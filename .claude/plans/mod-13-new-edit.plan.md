# Plan: MOD-13 milestone 2 — New and edit

**Source PRD**: `.claude/prds/mod-13-backlog-editing.prd.md`
**Selected Milestone**: 2 — New and edit
**Complexity**: Large
**Status**: fact-checked (step 3.5, 25 claims, amendments A1–A16 applied), awaiting CONFIRM

## Summary
This milestone adds two Backlog actions over the seven `version`-covered spec columns (ANA-9 §4.2):
`N` creates a new item, and `e` edits the selected item. Both stores already implement the write
half. `WriteStore::mint_item` is §7.1. `WriteStore::update_item` is the §7.2 compare-and-set with
`UpdateOutcome::Diverged`.

The rest of the chain is missing:
- worker requests;
- one shared input validator, because neither store checks title, tags, `touched_paths` or
  step-graph ownership;
- a capturing form in the Backlog;
- the offline refusal.

The three-way divergence view is milestone 3. In milestone 2, a stale edit keeps its text and is
never written over the head (D6).

## Design decisions
- **D1: one worker module, `crates/htui/src/item_writes.rs`, mirroring `requirements.rs`.**
  - Three new `StoreRequest`s:
    - `ItemForm { project, item: Option<ItemId> }`, the read that opens the form;
    - `MintItem { .. }`;
    - `EditItem { .. }`.
  - Three new `StoreReply`s:
    - `ItemForm(Box<ItemFormContext>)`;
    - `ItemWritten { item, outcome }`, self-naming as in MOD-59;
    - `ItemDiverged(Box<ItemDivergence { head, ancestor }>)`.
  - The worker fills in identity, as `requirements.rs` does: `created_by`/`author_id` from
    `backend.this_user()`, `box_id` from `backend.box_info()?.map(|b| b.box_id)`, and the id from
    `ItemId::new()` (UUIDv7).
  - **Redaction (A9):** `body` and `touched_paths` travel in a newtype whose `Debug` prints lengths
    only (`store_worker.rs:99-105` rule; `RequirementText` pattern, `requirements.rs:69-88`).
  - `StoreRequest` never carries a raw `ItemPatch` or `NewItem`. Both derive `Debug` over `body`.
    Titles stay plain `String`, as `CreateRequirementArea.title` is.
- **D2: offline is refused by the worker, and that is the gate.**
  - Each of the three requests first calls `backend.writer()`. With no writer it answers
    `Unreachable(DATABASE_UNREACHABLE)`. This includes `ItemForm`, so an offline box never opens a
    form.
  - `App::on_reply` already turns every non-stale `Failed` into
    `Action::Error("{request}: {message}")` (`app/update.rs:285-287`). The status line therefore
    shows `item_form: store unreachable: … this box browses its read-only cache and starts no run`.
    That is the visible read-only notice.
  - **The tab emits no second `Action::Error` (A8).** It only clears its `busy` state, and a write
    in flight also gets the notice line inside the form.
  - The Backlog has no `writable` flag, and there is no action registry. A worker-side gate cannot
    be bypassed by any view. So the PRD risk row "gate at the action registry" becomes "gate at the
    worker, plus a test per action".
- **D3: `ItemForm` is a fresh read through the writer (A1).** `item_kinds`, `step_graphs` and
  `repos` live on `WriteStore`, not `ReadStore` (`traits.rs:760, 818, 854`). D2's writer-first
  rule gives the worker that handle. The read returns:
  - the project's kinds;
  - its step graphs, **minus `is_override` graphs (A3)**, which are per-item clones that
    `step_graphs` does not filter (`kind.rs:142`);
  - its repos, with name and `is_primary`;
  - for an edit, the current `Item` (`ReadStore::item`), whose `version` is the compare-and-set
    token.

  Reading fresh means the token is always the version the form's text came from, never the Body
  sub-tab's possibly older copy.
- **D4: one shared validator in `htui-core`, `model/item_spec.rs`.** The worker calls it as the
  authority. The form calls it for early feedback, against the `ItemForm` context.
  - **Title**: trimmed, and not blank.
  - **Priority**: parses as `i16`.
  - **Required tags**: `declared_tags_from_text` (`box_.rs:281`). These are the box-tag rules; every
    demo tag passes (verified).
  - **Kind**: one of the project's kinds.
  - **Step graph**: `None` (the kind's default), or one of the project's non-override graphs. An
    override graph is accepted only as the item's unchanged current value. This closes the
    Mem/Pg gap: Mem accepts any id, and Pg checks only that it exists, not which project it is in.
  - **`touched_paths`**: one entry per line, trimmed at the line ends, blanks dropped, duplicates
    dropped with order kept.
    - The qualifier split is **`PathPrefix::parse`'s own rule**, factored out of
      `prompt/excerpt.rs:70-75` as `split_qualifier`. It is **not** `SkillGlob::parse`'s split,
      which trims and excludes metacharacters (A2).
    - Whitespace next to the `:` is refused (A2). `web: src/**` would otherwise store a glob with a
      leading space that never matches.
    - A qualified entry must name one of the project's repos.
    - A bare entry needs the project to have a primary repo. Without one, `overlap::resolve` drops
      it silently (`overlap.rs:55`).
    - The glob must compile under `skill_glob`'s `matcher` builder, which becomes `pub(crate)`.
    - The glob must not be empty after the `:`, must not start with `/`, and must contain no NUL.
    - Each refusal names the entry.
  - **On an edit, only the changed fields are validated (A4)**, so an item with legacy values can
    still have its title fixed.
- **D5: an edit sends only the changed fields.**
  - The form diffs against the `Item` from `ItemForm` and puts `Some` only on the fields that
    changed.
  - With no change it refuses with "nothing to save" and sends nothing.
  - Both stores bump `version` and write a revision on an all-`None` patch (`mem.rs:1478`,
    `pg/write.rs:838`), so the worker also refuses an empty patch.
  - `reason = "edited"`.
  - Re-kinding keeps the key; `update_kind_keeps_key_and_project` already pins this.
- **D6: in milestone 2, `Diverged` keeps the form and never overwrites.**
  - The reply carries `head` and `ancestor`, so milestone 3 can render its view from it.
  - The form shows "changed elsewhere — now v{head}; your text is kept. Esc and `e` reopen on the
    head". Milestone 3 adds the view and `divergence_resolution`.
  - It deliberately does **not** move the token to `head.version`. The Requirements tab does move
    it ("Ctrl+S saves over it", `requirements/mod.rs:139-144`), but an item's `R-ENT-10` forbids
    that silent overwrite. A second save diverges again.
- **D7: keys `N` (new) and `e` (edit).**
  - Plain `n` is taken. It falls through to Prompt's "next template" (`detail/prompt.rs:326`).
  - `N` and `e` are unbound everywhere. `N` with SHIFT reaches the list's match, because the list
    diverts only CONTROL|ALT (`backlog/mod.rs:334`).
  - `N` opens the form on the selected item's project, or on the first project in scope when
    nothing is selected. On a new item, the project is a picker. On an edit, it is fixed.
  - `e` with nothing selected does nothing.
  - Help rows go in `app/mod.rs` next to `f`/`F`.
- **D8: the form captures input and renders in the detail pane's `right` rect.** That rect is
  `panes(area)` at `backlog/mod.rs:428`, in place of `detail::render`. The fields do not fit the
  list pane's 7-row filter strip.
  - **Fields**:
    - project (new only);
    - kind;
    - title (`TextField`);
    - priority (`TextField`);
    - required tags (`TextField`, comma-separated);
    - step graph (picker: "kind default" plus the project's non-override graphs);
    - touched paths (`TextArea`, one per line);
    - body (`TextArea`).
  - **Keys**:
    - Tab/BackTab cycle focus.
    - **Ctrl+S is checked before chords are passed on (A6).** The filter's handler passes every
      chord at `mod.rs:248`, and `TextField` returns `Pass` on chords. Mirror
      `requirements/mod.rs:231, 1022`.
    - Enter on a `TextField` moves focus to the next field. It does not save.
    - Esc cancels.
    - While a write is in flight, `busy` swallows keys.
  - **Wiring (A7)**: the field is `item_form`, because `form` is already the filter form. The
    capture guards extend to the item form in `on_key`, in `on_paste` (`:315`), in the reveal
    guard (`:468`, `CLOSE_THE_FIELD_FIRST`) and in `on_scope_change` (`:305`), which closes it.
  - The form has a notice line for validation refusals, D6 and A12.
- **D9: re-read after a write lands.**
  - **Edit (A5):** close the form, then call a new `read_item(id)` helper. It is factored out of
    `go()`, which returns early on an unchanged selection (`mod.rs:148-150`), and re-sends the
    per-item reads, so the Body sub-tab shows the new version. The list rows are re-read under the
    active filter.
  - **Mint (A5):** close the form, then go through the reveal path (`mod.rs:462-483`). A just-minted
    item is never in the loaded rows, so **a mint always clears the active filter**, then selects
    the new item.
- **D10: one new conformance parity case, `update_spec_columns_roundtrip` (A10).**
  - It patches `priority`, `touched_paths`, `required_tags` and `step_graph_id`, using the fixture
    graph `ids::GRAPH_HTUI_*`. It sets them, then clears the graph with `Some(None)`.
  - It asserts on the **head** from `Updated`, because `ItemRevision` stores only title, body and
    tags (`item.rs:291-310`).
  - It updates the two case counts and their history messages (`mem_store.rs:37-56`,
    `pg_conformance.rs:18-29`).
  - Validator parity sits above the store, so it is pinned by `tests/item_writes_pg.rs` through
    `store_worker::serve`.
- **D11: a mint `Failed` is hedged, never called a refusal (A12, MOD-59 review M1).** When a
  COMMIT lands but its answer is lost, it comes back as `Failed`, and a retry would mint a
  duplicate. The form keeps its text and shows "it may have been written; the list is being
  re-read, look for it before Ctrl+S". The tab re-reads `Items`. The pattern is `mint_failed`
  (`requirements/mod.rs:126-136`).

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Worker write module | `crates/htui/src/requirements.rs` (`serve` 483, `write_access` 710, `offline` 715) | Writer first, then input checks, then the write; identity filled from `Backend` |
| Redacting text newtype | `requirements.rs:69-88` `RequirementText`; rule at `store_worker.rs:99-105` | Prose in a newtype whose `Debug` prints the length |
| Self-naming write reply | `store_worker.rs` `RequirementWritten`/`RequirementsStale` (1227-1236) | The applied write answers its own variant; the tab lands a write on that variant alone |
| Worker routing | `store_worker.rs` `try_serve` or-arm (1600-1609), `name()` (860) | A new or-arm; no other exhaustive match exists in the crate (verified) |
| Capturing form | `ui/tabs/requirements/forms.rs` (`RequirementForm`, `TextArea`, `FormFocus`), `ui/tabs/backlog/filter.rs` (`FilterForm`, `FormOutcome`) | Form state plus an outcome enum; Ctrl+S checked before the chord pass |
| Mint hedge | `requirements/mod.rs:126-136` `mint_failed` | The form keeps its text, then a re-read |
| Tag canonicalisation | `htui-core/src/model/box_.rs:281` | `Result<Vec<String>, String>`, refusal names the tag |
| Qualifier split | `htui-core/src/prompt/excerpt.rs:70-75` | Shared fn; **not** `SkillGlob::parse`'s split |
| Glob compile | `htui-core/src/model/skill_glob.rs:226-238` `matcher` | `globset` (non-optional dep) with the same flags |
| Tab unit tests | `ui/tabs/requirements/mod.rs` tests (`Bench`, `requests`, `save`), `backlog/mod.rs` tests | Drive keys, assert on emitted requests and notices |
| Worker tests | `requirements.rs` tests (`demo()`, offline test 1973) | Memory and `Backend::Offline { cache: CacheStore::open(..) }` |
| Integration | `tests/backlog.rs` (offline harness 1542-1586), `tests/requirements.rs:355-371` (stale via a held `MemStore` clone) | `Harness::over(store.clone())`, then write through `store` |
| Pg via worker | `tests/requirements_pg.rs` (`#![cfg(feature = "testkit")]`, `testkit::demo_db()` SKIP without DSN) | No harness; `store_worker::serve` over Online |

## Files to Change
| File | Action | Why |
|---|---|---|
| `crates/htui-core/src/model/item_spec.rs` | CREATE | D4 validator and its unit tests |
| `crates/htui-core/src/model/mod.rs` | UPDATE | `pub mod item_spec;` plus re-exports |
| `crates/htui-core/src/prompt/excerpt.rs` | UPDATE | `split_qualifier`, called by `PathPrefix::parse`; behaviour unchanged |
| `crates/htui-core/src/model/skill_glob.rs` | UPDATE | `matcher` becomes `pub(crate)` |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | D10 case in `CASES` and `run_case` |
| `crates/htui-core/tests/mem_store.rs` | UPDATE | Count 118 → 119 and its history message |
| `crates/htui-store/tests/pg_conformance.rs` | UPDATE | `EXPECTED_CASES` 118 → 119, doc and message |
| `crates/htui/src/item_writes.rs` | CREATE | D1–D6, D11 serve fns, redacting newtypes, worker unit tests |
| `crates/htui/src/lib.rs` | UPDATE | `pub mod item_writes;` between `hierarchy` and `keymap` |
| `crates/htui/src/store_worker.rs` | UPDATE | 3 requests, 3 `name()` arms, 3 replies, 1 or-arm |
| `crates/htui/src/ui/tabs/backlog/item_form.rs` | CREATE | D8 form: state, keys, render, diff-to-patch (D5), notice line |
| `crates/htui/src/ui/tabs/backlog/mod.rs` | UPDATE | `N`/`e`; the `item_form` field and guards (A7); `read_item` factored out of `go()` (A5); `on_reply` for the 3 replies plus `Failed`; render in `right` |
| `crates/htui/src/app/mod.rs` | UPDATE | Help rows for `N`/`e` |
| `crates/htui/tests/backlog.rs` | UPDATE | Integration cases; a `backlog_over_agent(store)` helper (A13); an offline helper that seeds mirror item rows through `cache.pool()` (A11) |
| `crates/htui/tests/snapshots/backlog__item_form_*.snap` | CREATE | New-form and edit-form snapshots |
| `crates/htui/tests/item_writes_pg.rs` | CREATE | Pg parity through `store_worker::serve` |

**Every task:** each new public item gets a doc comment and each public type a `Debug`, because
of `#![warn(missing_docs)]` in `lib.rs:10`, the `missing_debug_implementations` workspace lint,
and `-D warnings` (A14).

## Tasks
### Task 1: shared validator (TDD), `htui-core`
- **Action**: Write the tests first:
  - title: blank or whitespace is refused;
  - priority: `"x"`/`"40000"` are refused, `"-3"` is accepted;
  - tags: `"Rust"` is refused by name, and `"rust, docker,rust"` becomes `["docker","rust"]`;
  - touched paths, against an explicit repo list in each test, **never demo data, which has no
    repos (A15)**:
    - bare `src/**` with a primary repo is kept;
    - bare with no primary is refused;
    - `web:src/**` with repo `web` is kept;
    - `nope:src` is refused, naming `nope`;
    - `web:` is refused;
    - `web: src` is refused;
    - `/abs` is refused;
    - `src/[` is refused;
    - `a/b:c` is bare;
    - blanks and duplicates are dropped with order kept;
  - kind: one outside the project is refused;
  - graph: one outside the project is refused, an override is refused unless unchanged, and
    `None` is accepted;
  - edit-mode validation checks only the changed fields.

  Then factor out `split_qualifier` (the existing `PathPrefix` tests stay green), expose
  `matcher`, and implement.
- **Files**: `model/item_spec.rs`, `model/mod.rs`, `prompt/excerpt.rs`, `model/skill_glob.rs`.
- **Validate**: `cargo test -p htui-core --all-features --lib item_spec excerpt skill_glob`

### Task 2: conformance parity case (TDD)
- **Action**: Add `update_spec_columns_roundtrip` (D10) and bump both counts and their messages.
- **Files**: `store/conformance.rs`, `htui-core/tests/mem_store.rs`, `htui-store/tests/pg_conformance.rs`.
- **Validate**: `cargo test -p htui-core --all-features --test mem_store`;
  `cargo test -p htui-store --all-features --test pg_conformance -- --test-threads=1`

### Task 3: worker requests (TDD), `htui`
- **Action**: Write the tests first in `item_writes.rs`, over `Backend::memory(MemStore::demo())`.
  Where a path is accepted, the test first creates a repo with `create_repo`.
  - `ItemForm` (new) returns kinds, non-override graphs and repos, with `item: None`.
  - `ItemForm` (edit) carries the `Item`.
  - `MintItem` lands `KEY-n+1` with revision 1 and answers `ItemWritten { Minted }`.
  - `EditItem` with a changed title lands `version + 1` with reason `edited`.
  - A stale `EditItem` answers `ItemDiverged { head, ancestor }` and writes nothing.
  - An empty patch is refused, and nothing is written.
  - Each D4 refusal answers `Failed` naming the entry, and nothing is written.
  - Offline (`Backend::Offline`), all three answer `Failed` with `DATABASE_UNREACHABLE`.
  - The `Debug` of `MintItem`/`EditItem` holds neither the body nor the paths.

  Then add the variants, the names and the or-arm.
- **Files**: `item_writes.rs`, `lib.rs`, `store_worker.rs`.
- **Validate**: `cargo test -p htui --all-features --lib item_writes store_worker`

### Task 4: Backlog form and wiring (TDD), `htui` UI
- **Action**: Write the unit tests first:
  - `N` emits `ItemForm { item: None }` on the selected project, both with and without SHIFT
    (A16).
  - `e` emits `ItemForm { item: Some(id) }`; `e` with nothing selected emits nothing.
  - The `ItemForm` reply opens the form, and the form captures (`j` does not move the list).
  - Paste goes to the form.
  - Ctrl+S on a blank title shows the refusal in the form and sends nothing.
  - An unchanged edit shows "nothing to save" and sends nothing.
  - A changed title sends `EditItem` with only the title changed, at the `version` from the reply.
  - `ItemWritten` (edit) closes the form and re-sends the per-item reads with the selection
    unchanged.
  - `ItemWritten` (mint) closes the form, clears the filter and reveals the new item.
  - `ItemDiverged` keeps the form and its text, the token stays where it was, and the notice
    shows.
  - A mint's `Failed` keeps the text, shows the hedged notice and re-reads `Items`.
  - A `Failed` for `item_form` opens no form, and the tab emits no extra `Action::Error`.
  - A scope change closes the form, and a reveal while it is open is refused.
  - With no form open, nothing on screen changes.

  Then implement `item_form.rs`, the `mod.rs` wiring and the help rows.
- **Files**: `ui/tabs/backlog/item_form.rs`, `ui/tabs/backlog/mod.rs`, `app/mod.rs`.
- **Validate**: `cargo test -p htui --all-features --lib backlog`

### Task 5: integration, snapshots, Pg parity
- **Action**: In `tests/backlog.rs`, add these cases:
  - `N`, a title, Ctrl+S: the list shows the new key, selected.
  - `e`, a changed title, Ctrl+S: Body shows `v2`.
  - A stale edit: hold a `MemStore` clone and write through it between open and save (A13). The
    notice shows, the form stays open, and the head is unchanged.
  - An unknown repo in touched paths: the refusal names it, and nothing is written.
  - Offline with mirror item rows seeded (A11), for both `N` and `e`: the status line ends with
    `DATABASE_UNREACHABLE`, no form opens, and nothing is written.
  - The snapshots `backlog__item_form_new` and `backlog__item_form_edit`. Every existing
    `backlog__*.snap` stays untouched.

  In `tests/item_writes_pg.rs`: mint, edit, diverge and a refusal through `store_worker::serve`
  over Postgres, with the same outcomes as the Memory unit tests.
- **Files**: `tests/backlog.rs`, the new snapshots, `tests/item_writes_pg.rs`.
- **Validate**: `cargo test -p htui --features testkit --test backlog -- --test-threads=1`;
  `cargo test -p htui --all-features --test item_writes_pg -- --test-threads=1`

**Task order: serial, T1 → T2 → T3 → T4 → T5.**
- The draft marked T1 ∥ T2. Their file sets are disjoint, but both build the `htui-core` crate.
  On one tree, T2's gate compiles T1's half-written module.
- A worktree split would buy minutes on a small task at ~10G of `target/`, so it was dropped.
- T5's `item_writes_pg.rs` shares no file with T4, but it links the lib that T4 is editing, so it
  stays in T5.
- Ultracode runs this as a serial implement → verify pipeline per task.

## Validation
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p htui-core --all-features
cargo test -p htui --all-features -- --test-threads=1
cargo test --workspace --all-features --no-fail-fast -- --test-threads=1   # full gate; grep SIGABRT
```

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| A stale edit is overwritten by a second save | M | D6: the token never moves, pinned by unit and integration tests. M3 replaces this branch |
| A lost mint answer leads to a duplicate on retry | L | D11 hedge and re-read |
| Validator and `overlap::resolve` classify a qualifier differently | M | A shared `split_qualifier`; tests pin `a/b:c` and `web: src` |
| An item with legacy tags or paths cannot be edited at all | L | A4: only the changed fields are validated |
| A body leaks through `Debug` | L | A9 newtype, plus a test on the request's `Debug` |
| An empty patch bumps `version` | M | D5: refused by both the form and the worker |
| Snapshot churn | L | The form renders only when open; existing snaps stay untouched |
| `htui-orch` stack headroom | L | No orchestrator change; the gate runs `--no-fail-fast` and greps SIGABRT |

## Verified claims
| # | Claim | Verdict | Evidence |
|---|---|---|---|
| C1 | `PathPrefix::parse`'s qualifier split factors out unchanged; `a/b:c` is bare | ✓ | `prompt/excerpt.rs:70-75`; other callers `htui-agent/src/excerpt.rs:947`, `model/overlap.rs:227` |
| C2 | `overlap::resolve` uses the same rule; it drops a bare entry with no primary and errors on an unknown repo | ✓, amended (A2) | It calls `PathPrefix::parse` (`overlap.rs:51`), drop at `:55`, `UnknownTouchedRepo` at `:59-60`. `SkillGlob::parse` (`skill_glob.rs:42-51`) splits differently, so it is not the mirror |
| C3 | `skill_glob`'s builder can become `pub(crate)`; `globset` is always compiled | ✓ | `matcher` at `skill_glob.rs:226-238`; `htui-core/Cargo.toml:23` non-optional |
| C4 | `declared_tags_from_text` is `&str -> Result<Vec<String>, String>` and names the tag | ✓ | `box_.rs:281, 292-296` |
| C5 | `ReadStore` has `item_kinds`/`step_graphs`/`repos`/`item` | ✗ (A1) | Only `item` (`traits.rs:92`); the other three are `WriteStore` (`:760, 818, 854`). `step_graphs` includes `is_override` clones (A3) |
| C6 | `ItemId::new()` is UUIDv7 | ✓ | `model/ids.rs:31-32` |
| C7 | An all-`None` patch bumps `version` and writes a revision on both stores | ✓ | `mem.rs:1478`; `pg/write.rs:838`. A revision holds only title, body and tags (`item.rs:291-310`), hence A10 |
| C8 | Conformance `CASES`/`run_case`; counts 118 in `mem_store.rs` and `pg_conformance.rs` | ✓, amended | `conformance.rs:48, 178`; `mem_store.rs:37` and its message `:38-56`; `pg_conformance.rs:22` with doc and message `:18-29`. `htui-agent`/`htui-orch` suites are separate |
| C9 | The parity case needs a real step graph (Pg FK; Mem accepts anything) | ✓, amended | Fixture `ids::GRAPH_HTUI_*` (`fixtures.rs:195-203`), so no graph is created; `0001_init.sql:324` |
| C10 | T1 ∥ T2 | ✗, struck | Disjoint files, but a shared `htui-core` build; now serial |
| C11 | No demo item carries a tag the validator refuses | ✓ | `fixtures.rs:925-1055`: rust, docker, gpu, vulkan, cmake |
| C12 | No demo `touched_paths` value would be refused | ✓, with a caveat (A15) | All empty (`fixtures.rs:1078`), but `DemoData` has **no repos** (`fixtures.rs:344-404`), so tests create them |
| H1 | `requirements.rs` serve, writer and offline shape; `this_user`, `box_info` | ✓ | `requirements.rs:483, 710, 715`; `backend.rs:170, 252` |
| H2 | Only `store_worker.rs` needs new arms | ✓ | `name()` `:860`, `try_serve` `:1478`; every other match has a fallback (`app/update.rs:383`, `store_worker.rs:2258`, `testkit.rs:362, 534`, `run_worker.rs:129`, `agent_worker.rs:1057`, `concepts_worker.rs:397`) |
| H3 | No redaction concern | ✗ (A9) | Rule at `store_worker.rs:99-105`; `ItemPatch` derives `Debug` over `body` (`item.rs:263`) |
| H4 | `N`/`e` are unbound; `n` is taken; SHIFT+`N` reaches the list | ✓ | No `Char('N')`/`Char('e')` in `backlog/**`, `keymap.rs`, `app/mod.rs`; `n` at `detail/prompt.rs:326`; diversion only for CONTROL\|ALT at `mod.rs:334` |
| H5 | The detail pane has its own rect; the reveal guard and scope-change close exist | ✓ | `panes(area)` `mod.rs:428`, `detail::render` `:453`; `:468-469`; `:305` |
| H6 | A just-minted item can be selected through reveal | ✓, amended (A5) | `reveal` `mod.rs:462`. A miss always clears the filter (`:479-483`); the guard checks only the filter form |
| H7 | `go()` refreshes an unchanged selection | ✗ (A5) | `if next == self.selected { return; }` at `mod.rs:148-150` |
| H8 | `TextArea`/`TextField` API | ✓, amended (A6) | `text_area.rs:191-199, 94, 108`. `TextField` passes chords (`text_field.rs:46-55`), so Ctrl+S is checked first |
| H9 | A test can bump the version mid-form | ✓, amended (A13) | No store accessor on `Harness`; use a held clone (`tests/requirements.rs:355-371`) |
| H10 | The offline harness has an item to select | ✗ (A11) | `seed_mirror` writes no items (`tests/backlog.rs:1539`); seed through `cache.pool()` (`cache/mod.rs:231`) |
| H11 | Pg suite gating | ✓, amended | `#![cfg(feature = "testkit")]` and `demo_db()` SKIP (`tests/requirements_pg.rs:11-13, 33`); no `[[test]]` entry needed |
| H12 | Lints a new module must meet | ✓, amended (A14) | `lib.rs:10` `missing_docs`; workspace `missing_debug_implementations` (`Cargo.toml:140-153`) |
| H13 | T5's Pg test ∥ T4 | ✗, struck | No shared file, but it links T4's in-flux lib; it stays in T5 |
| D2' | The tab should emit its own `Action::Error` on `Failed` | ✗ (A8) | `App::on_reply` already does (`app/update.rs:285-287`) |
| D11' | A mint's lost answer is covered | ✗ (A12) | MOD-59 M1 hazard; `mint_failed` at `requirements/mod.rs:126-136` |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes
- [ ] Patterns mirrored, not reinvented
- [ ] No existing snapshot changed
- [ ] Offline: `N`/`e` refused with the read-only notice and no write
- [ ] A stale edit never overwrites the head

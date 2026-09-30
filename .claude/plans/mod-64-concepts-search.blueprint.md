# Blueprint: MOD-64, concepts search in the TUI

**Status**: accepted (2026-09-29); F1 resolved by the maintainer as `find` (D236, D257 amended). Findings F1–F13 (§0) and decisions D241–D262 (§11) are proposed **Superseded at review (2026-09-30):** the model load and the Qdrant connection no longer follow D245, F9 and §3.2's `OnceCell` design — one shared in-flight load (retried once after a failure) and a connection cached by URL and key (review findings 2 and 3; plan D238 as amended).
here. **Blocker** means the plan, read literally, fails its own acceptance or its own named test, or
leaves the gate red. The Fix column is what the implementer builds.

**Plan**: `.claude/plans/mod-64-concepts-search.plan.md`, confirmed 2026-09-29 with the OQ-1..OQ-4
defaults (commit `9a1cf27`). D230–D240, T1–T5 and the file sets are authoritative except where §0
amends them. None of D230–D240 is reopened; where the tree forces a choice the plan left open or
stated two ways, §0 says so and §11 records it.

**Verified at**: HEAD `9a1cf27`, branch `hr/MOD-64`. `git diff --stat b5e481f HEAD -- crates
Cargo.toml Cargo.lock` is empty, so the plan's fact-check still holds. Every symbol below was read in
full through the Gortex graph (`read source` / windowed `read file`). **Line numbers are pre-edit**:
a citation into a file a task edits moves after that task's first commit. `df -h /` shows 101 GB free.

**House style (carried)**:
- `unsafe_code = "forbid"`. Lib roots warn on `missing_docs`; the workspace warns on
  `missing_debug_implementations` and `unused_qualifications`; rustdoc denies
  `broken_intra_doc_links`, `private_intra_doc_links`, `redundant_explicit_links`
  (`Cargo.toml` `[workspace.lints.*]`). Clippy `all` is `warn` and the gate runs `-D warnings`;
  `pedantic` is off. `rustfmt.toml`: `max_width = 100`.
- Anything holding user text gets a hand-written `Debug` only where it may hold a secret
  (`TextField`'s rule, `ui/text_field.rs:56-64`); a search query is not one, so derives are fine.
- No `std` guard across an `.await`.
- Implementers commit incrementally, staging their own paths only (never `-A`, never `stash`, never
  `--amend`). Every commit compiles; a red commit uses `todo!()` bodies.
- **No intra-doc link (`[`X`]`) from T1 to an item T3 creates or back**: they run in parallel
  worktrees and each runs `cargo doc` alone. Name such items in plain backticks.
- Every gate re-runs with `--test-threads=1` on the real tree after a merge (the keyring fake is one
  process-wide slot, `htui-store/src/testkit.rs:273`).

---

## 0. Findings the plan fact-check missed

| # | Severity | Plan says | Tree at `9a1cf27` | Fix |
|---|---|---|---|---|
| **F1** | Major (gate: 27 snapshots move) | D236: global `Ctrl+F`, help text `search concepts`, "bound in `register_all` like `w`". Nothing about snapshots. | `App::render` draws `keymap.help_line(&KeyScope::Global)` on the status row whenever `status` is `None` (`app/state.rs` `render`). Today that line is 85 cells: `q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ? help · w workspaces`. Appending ` · Ctrl+f search concepts` (`KeyChord::label` prints `Ctrl+f`, `keymap.rs:88-100`) makes it 110; the Harness is 100 wide (`testkit.rs:34`), so every registered-shell snapshot's last row changes and ends `· Ctrl+f search c`. 27 files carry that row: 26 under `crates/htui/tests/snapshots/` (list in §5.9) and `crates/htui/src/snapshots/htui__testkit__tests__shell_empty.snap`. No test asserts the whole line (`keymap.rs:491` pins `default_global` only; `tests/integration.rs:58` checks `contains("w workspaces")`, still true). | **Resolved by the maintainer 2026-09-29: `find`.** D257: T4 re-blesses exactly those 27 in the commit that adds the binding, and checks that only their **last line** changed (§5.9 gate). The help text stays `search concepts` (D236 is settled). If the maintainer would rather the row fit at 100 columns, `find` (` · Ctrl+f find`, 99 cells) is the only short text that does; that is a D236 amendment for the maintainer, not an implementer's call. |
| **F2** | Blocker (the overlay cannot read an error) | D231: no-runtime answer `Concepts { outcome: Err(..) }`. D232: `StoreReply::Concepts(ConceptsReply)`, `ConceptsReply::{Hits { hits, query }, Indexed(SyncReport)}` "wrapped in `Result<_, String>`". | The two shapes disagree. Either way, an `Err(String)` does not say which request failed, and both requests come from the same `Origin::Overlay(id)`. `App::latest` keys freshness by `(Origin, Discriminant<StoreRequest>)` (`state.rs:170`), so a search reply and an index reply are both fresh at once, and the overlay would not know whether to put an error on the search line or the index line. | D242: the error lives **inside** each kind: `StoreReply::Concepts(Box<ConceptsReply>)`, `ConceptsReply::Hits { query: SearchQuery, outcome: Result<Vec<Hit>, String> }` and `ConceptsReply::Indexed(Result<SyncReport, String>)`. D232's intent (one variant, error inside, never `Failed`, the query echoed) is kept. |
| **F3** | Major (harness is scheduling-dependent) | D231: "`Harness::with_concepts_runtime` mirrors `with_agent_runtime`; `drive`/`settle` route the two requests to it." | `drive` only delivers what reached `replies.1` by the time it looks (`testkit.rs:364-367`); a `tokio::spawn`ed search may not have run yet. The run runtime solves this by awaiting its tasks every round (`RunRuntime::settle`, `run_worker.rs:1632-1655`, called at `testkit.rs:344-351`). Neither precedent routes through `settle`: the agent runtime is `drive`-only (`testkit.rs:276-306`) and `with_run_runtime`'s doc says "`Harness::settle` stays runtime-free" (`testkit.rs:156-158`); `settle` never reads `replies.1` (`testkit.rs:484-516`), so a spawned task's reply could never be delivered by it. | D243/D244: `ConceptsRuntime::settle(limit) -> usize` (harness-only, the `RunRuntime::settle` shape) and `drive` calls it every round. `settle` stays runtime-free and answers both requests through `store_worker::serve`, i.e. the no-runtime `NOT_AVAILABLE` reply. Concepts tests use `drive`/`drive_to_end`. |
| **F4** | Major (a named test cannot pass) | D235: `RevealTarget::{Item(ItemId), Requirement(RequirementId)}`; "an item absent from the reply says `<KEY> is not in this workspace's backlog`". T3 test "an unknown id → the notice". | When the item is absent, the Backlog has no row to read the key from. | D248: each target carries its key: `RevealTarget::Item { id, key }`, `RevealTarget::Requirement { id, key }`. The overlay takes it from `Hit::key` (`vector.rs:328`), which is the owner item's key on a document hit. |
| **F5** | Minor | D235: "on the tab's notice". | `BacklogTab` has no notice or status field (`backlog/mod.rs:39-48`: `items`, `folded`, `selected`, `detail`) and no row to draw one; adding one moves every Backlog snapshot's layout. `RequirementsTab` has `notice: Option<Notice>` (`requirements/mod.rs:187`). | D249: the Backlog emits `Action::Error(not_in_this_backlog(key))`, which the status line shows until the next key (the `Action::Error` precedent of `app/mod.rs:94-99` and `requirements/mod.rs:399-414`). Requirements uses its own `Notice::Error`. |
| **F6** | Minor (misleading, compiles) | D235: "the same with its existing private `reveal`". | `RequirementsTab::reveal(&mut self, row: Row, ctx: &Ctx<'_>)` (`requirements/mod.rs:801-819`) has the new trait method's name. Inherent methods win method resolution, so `self.reveal(row, ctx)` at `:788` still compiles, but two `reveal`s of different arity on one type is a trap for the next reader. | D253: rename the private one to `unhide` (one caller, `:788`); behaviour unchanged. |
| **F7** | Blocker (acceptance 1 fails) | Acceptance 1: "`Ctrl+F` opens the search from every tab". Fact-check: "no `Char('f')` binding anywhere". | With the Chat composer open, `Composer::on_key` pushes every `KeyCode::Char` regardless of modifiers and answers `Consumed` for anything else (`chat/composer.rs:59-86`); `ChatTab::on_key` returns on that before any shell binding (`chat/mod.rs:474-482`). `Ctrl+F` types an `f`. (The same eats `Ctrl+C`, which MOD-52's doc at `keymap.rs:199-203` says every text field passes; pre-existing.) Every other text widget passes chords: `TextField::on_key` `:119-128`, `TextArea::on_key` `:190-199`. | D256: T4 adds, first thing in `ChatTab::on_key` after `self.refusal = None;` (`chat/mod.rs:463`), `if key.modifiers.intersects(KeyModifiers::CONTROL \| KeyModifiers::ALT) { return Handled::Pass; }`. The Chat tab binds no chord (`chat/mod.rs:460-525`, `transcript.rs:445-482`). T4's file set gains `ui/tabs/chat/mod.rs`. |
| **F8** | Major (the D240 test cannot go red) | T3 test: "D240: close an overlay with a request in flight, reopen it, the old reply is dropped" (harness over demo, `tests/reveal.rs`). | Before T4 the only overlays are the switcher and the migration prompt; the switcher re-requests `Workspaces` on every open (`workspace_switcher.rs:155-158` via `push_overlay`, `state.rs:280-288`), which overwrites `latest` and hides the bug. A harness test written with it passes before the fix. | D247: the D240 tests go in `app/update.rs`'s `mod tests` (the `Recorder`/`Popup` scaffold, `update.rs:375-951`) with a test overlay that requests on a key and never on open. T4 adds the end-to-end one over the real overlay (§5.8, `a_reply_to_a_closed_search_is_not_shown_by_a_reopened_one`). |
| **F9** | Minor (supersede can double-load the model) | D238: `OnceCell<FastEmbedder>` initialised through `spawn_blocking(FastEmbedder::new)`; D231: a new search aborts the superseded task. | If the cell's `get_or_try_init` future runs **inside** the search task, aborting a first search mid-load drops it; `spawn_blocking` cannot be cancelled, so the load keeps running while the next search starts a second one (a first-ever run downloads BGE-small twice into the same cache dir, `embed.rs:40-56`). | D245: `QdrantIndex` holds `Arc<OnceCell<FastEmbedder>>` and runs `get_or_try_init` on a **detached** `tokio::spawn` that the search awaits. An aborted search drops only the `JoinHandle`; the load finishes and fills the cell; a concurrent search waits on the same init (tokio's `OnceCell` serialises initialisers). A failed load leaves the cell empty (`get_or_try_init` caches only `Ok`). |
| **F10** | Minor | D234: "the first one says `loading the embedding model…`". | The overlay cannot see whether the process has loaded the model (it lives in the worker); a new overlay is built on every open (`OverlayRegistry::create`, `registry.rs:99-105`). | D254: "first" means the first search **this overlay instance** sends; later ones say `searching…`. A reopened overlay's first search may say `loading…` for an already-loaded model: a one-frame overstatement, accepted. |
| **F11** | Minor (test does nothing) | T1/Test plan: "`cargo test -p htui-store` for `embed.rs`". | `FastEmbedder` is `#[cfg(feature = "local-embed")]` (`embed.rs:23-26`), and `htui-store`'s own tests do not enable it (`htui-store/Cargo.toml` `[dev-dependencies]`). The only real-model test is `#[ignore]` (`embed.rs:170`). | D258: the `Clone` check is a compile-time assertion in `htui`'s `concepts.rs` tests (`htui` enables `local-embed`, `htui/Cargo.toml:31`). `cargo test -p htui-store` is still run, for the derive's compile. |
| **F12** | Minor (gate runs nothing new) | Gate: `cargo test --workspace -- --test-threads=1`. | `tests/reveal.rs` and `tests/concepts_search.rs` will be `#![cfg(feature = "testkit")]`, like every harness file (`tests/shell.rs:8`); without `--all-features` the gate compiles them empty. | §8: every `htui` gate command carries `--all-features`. |
| **F13** | Minor (compile hazard, T3) | T3 adds `pending_reveal` to `BacklogTab`. | `BacklogTab` has no `Default` derive and is built by struct literal in its own test `a_capturing_sub_tab_gets_the_navigation_keys` (`backlog/mod.rs:351-356`). `RequirementsTab` derives `Default` and its test builder uses `..RequirementsTab::default()` (`requirements/mod.rs:1243-1249`), so it needs nothing. | T3 adds `pending_reveal: None` to `BacklogTab::new` (`:66-71`) and to that literal. |

### 0a. Settled answers to the brief's questions

| Question | Answer | Where |
|---|---|---|
| `StoreRequest` shapes and names | `SearchConcepts(SearchQuery)` → `"search_concepts"`; `IndexConcepts { scope: Scope }` → `"index_concepts"`; `concepts_worker::REQUEST_NAMES` pins both. | D241, §3.3 |
| Reply payload; derives needed | `StoreReply` and `StoreRequest` derive only `Debug, Clone` (`store_worker.rs:100`, `:854`). `SearchQuery: Debug, Clone, PartialEq, Eq` (`vector.rs:301`), `Hit: Debug, Clone, PartialEq` (`:321`, `f32`, so no `Eq`), `SyncReport: Debug, Default, Clone, Copy, PartialEq, Eq` (`vector_sync.rs:32`). `ConceptsReply` derives `Debug, Clone, PartialEq`. Boxed in `StoreReply`, the `Templates(Box<..>)` precedent (`store_worker.rs:1007`). | D242 |
| How `Served`/`Deferred` is typed | `agent_worker::Served` carries a chat-only `Start` (`agent_worker.rs:347-357`), so reusing it forces a dead arm. The run runtime has its own `RunServed { Reply, Deferred, Attach }` (`run_worker.rs:557-570`). Mirror that: `ConceptsServed { Reply(StoreReply), Deferred }`, `#[derive(Debug)]`. `serve` is a plain `fn` (it spawns and awaits nothing). | D243 |
| Worker-loop dispatch today | `AgentRuntime::serve(&backend, &tx, &envelope).await` for 15 requests (`store_worker.rs:1860-1885`), `runs.serve(&backend, &tx, &envelope, &live).await` for 3 (`:1886-1896`), then `other => try_serve` (`:1897`). The concepts arm goes between the run arm and `other`. | §3.3 |
| Harness routing; no-runtime answer | `drive` matches `(request, &store_state)` pairs (`testkit.rs:255-334`); the concepts arm goes before `(request, _)` at `:333`, and falls back to `store_worker::serve` like the run arm does (`:318-320`). That reaches `try_serve`'s new arms: `StoreReply::Concepts(..Err(NOT_AVAILABLE))`, never `Failed`. | D244 |
| `try_serve` arms | Two explicit arms, no wildcard, after the Qdrant arm (`store_worker.rs:1384-1390`). | §3.3 |
| `BoxFuture` spelling | `futures` is a normal dependency of `htui` (`htui/Cargo.toml:33`, workspace `0.3`): `futures::future::BoxFuture<'a, T>` = `Pin<Box<dyn Future<Output = T> + Send + 'a>>`; bodies are `Box::pin(async move { .. })`. | §3.2 |
| `OnceCell` + `spawn_blocking`, failure not cached | §3.2 `QdrantIndex::embedder`; F9. | D245 |
| Where D240's cleanup lives | `App::forget(&Origin)` beside `is_fresh` in `state.rs`; `App::close_top_overlay` / `App::close_every_overlay` in `update.rs`, called from all three pop paths: `update.rs:86` (`Close`), `:87` (`CloseAll`), `:122` (`set_scope`). These are the only `overlays.pop()`/`overlays.clear()` calls in the crate. | D247, §4.2 |
| `Tab::reveal` signature; how its `Ctx` is built | `fn reveal(&mut self, _target: &RevealTarget, _ctx: &mut Ctx<'_>) -> bool { false }`. `focus_section` takes no `Ctx` and `update_tab` calls it through `tabs.by_id_mut` (`update.rs:62-67`); `reveal` needs one to issue reads, so `App::reveal` destructures `self` and builds it the way `App::finish_external_edit` does (`state.rs:361-390`), then `drain`s the tab's origin. | D250, §4.3 |
| How Backlog finds the project to unfold | `ItemSummary::project_id` (used at `backlog/mod.rs:241`); the fold set is `folded: Vec<ProjectId>` and `list::rows` hides a folded project's items (`list.rs:77-90`). | §4.4 |
| Backlog notice | None exists; `Action::Error`. | F5, D249 |
| Requirements `select_row` | `land`'s tail, `requirements/mod.rs:788-797`, moved verbatim into `select_row(&mut self, row: Row, ctx: &Ctx<'_>)`; `land` keeps `:784-787` and calls it. | D253, §4.5 |
| Overlay layout, constants, snapshot names | §5.3–§5.5, §5.8. | D254, D255 |

---

## 1. Build order and validation, at a glance

| Task | Files | Commits (each compiles) | Gate |
|---|---|---|---|
| T1 shared formats | `htui-store/src/embed.rs`, `htui/src/concepts.rs` (worktree A) | 2 (§2.4) | `cargo test -p htui --all-features --lib concepts`; `cargo test -p htui-store`; clippy `-p htui -p htui-store` |
| T3 reveal + D240 | `app/{action,state,update,mod}.rs`, `ui/tabs/registry.rs`, `ui/tabs/backlog/mod.rs`, `ui/tabs/requirements/mod.rs`, `tests/reveal.rs` (worktree B) | 5 (§4.8) | `htui` lib tests; `--test reveal`; `--test requirements --test backlog --test integration`; clippy |
| merge | — | T1, then T3 | after each merge, its gate on the real tree |
| T2 worker | `concepts_worker.rs` (new), `lib.rs`, `store_worker.rs`, `testkit.rs`, `htui/Cargo.toml` | 2 (§3.7) | `htui` lib tests (keyring test under `--test-threads=1`); clippy; `git diff --exit-code Cargo.lock` |
| T4 overlay | `ui/overlay/concepts_search.rs` (new), `ui/overlay/mod.rs`, `app/mod.rs`, `ui/tabs/chat/mod.rs`, `tests/concepts_search.rs` (new), 5 new + 27 moved snapshots | 3 (§5.10) | whole `htui` suite; snapshot check §5.9 |
| T5 close-out | main thread, per the plan | — | the workspace gate (§8), validator |

T1 ∥ T3 (disjoint files). T2 needs T1 (`FastEmbedder: Clone`) and runs after T1's merge, alongside or
after T3's (T2 ∩ T3 = ∅; §7 lists the one shared type, none). T4 needs T2 and T3. The two parallel
worktrees share one `CARGO_TARGET_DIR` (MOD-9 D32; ~10 GB per cold target).

---

## 2. T1: `FastEmbedder: Clone` and the shared line formats (D238, D239)

**First failing test**: `concepts::tests::report_line_is_index_items_wording`.

### 2.1 `crates/htui-store/src/embed.rs`

At `:23-24`, between `#[cfg(feature = "local-embed")]` and `pub struct FastEmbedder {`, add
`#[derive(Clone)]`. The one field is `std::sync::Arc<fastembed::TextEmbedding>` (`:25`), so the clone
is a refcount bump. Extend the doc line at `:22`: `… run on the blocking pool. Cheap to clone: the
model is shared (MOD-64 D238).` Nothing else in the crate changes.

### 2.2 `crates/htui/src/concepts.rs`

- Import: `use htui_store::vector_sync::{Indexer, SyncReport};` replaces `:18`; `index_items`
  (`:130`) then says `SyncReport::default()`.
- After `DECISION_RESOLUTIONS` (`:40-44`), before `settings_from` (`:46`):

```rust
/// The search both front ends send (MOD-64 D239): `text` in `projects`, narrowed to decisions (items
/// closed as one of [`DECISION_RESOLUTIONS`], and their documents) when `decisions` is set, at most
/// `limit` hits. No type or status filter: `--search-items` has none either.
#[must_use]
pub fn query(text: &str, projects: Vec<ProjectId>, decisions: bool, limit: u64) -> SearchQuery {
    SearchQuery {
        text: text.to_owned(),
        projects,
        types: Vec::new(),
        statuses: Vec::new(),
        resolutions: if decisions { DECISION_RESOLUTIONS.to_vec() } else { Vec::new() },
        limit,
    }
}
```

- After `format_hit` (`:182-206`), before `mod tests` (`:208`):

```rust
/// What one index run did, as one line (MOD-64 D237, D239): `htui --index-items` prints it on
/// stderr and the search overlay under its hits.
#[must_use]
pub fn report_line(report: &SyncReport) -> String {
    format!(
        "indexed: {} item(s) rebuilt, {} unchanged; {} requirement(s) rebuilt, {} unchanged; \
         {} point(s) written, {} removed",
        report.items_rebuilt, report.items_unchanged, report.requirements_rebuilt,
        report.requirements_unchanged, report.points_upserted, report.points_deleted
    )
}
```

- `index_items` (`:128-146`): the `eprintln!(…)` block becomes `eprintln!("{}", report_line(&report));`.
- `search_items` (`:153-180`): the `SearchQuery { … }` literal becomes
  `let query = query(&options.query, projects, options.decisions, options.limit);` (the local shadows
  the fn after the call; if clippy objects, name it `search`). `SearchQuery` stays imported (the
  return type of `query`).

### 2.3 Tests (unit, `concepts.rs` `mod tests`, written first)

- `query_narrows_to_decisions_only_when_asked`: with `decisions = false`, `resolutions` is empty; with
  `true`, it equals `DECISION_RESOLUTIONS.to_vec()`. In both, `text`, `projects` (two ids, order
  kept) and `limit` are as passed, and `types`/`statuses` are empty.
- `report_line_is_index_items_wording`: `SyncReport { items_rebuilt: 1, items_unchanged: 2,
  requirements_rebuilt: 3, requirements_unchanged: 4, points_upserted: 5, points_deleted: 6 }` gives
  exactly `"indexed: 1 item(s) rebuilt, 2 unchanged; 3 requirement(s) rebuilt, 4 unchanged; 5
  point(s) written, 6 removed"`. The literal is copied from today's `index_items` format string, so
  the CLI's bytes are pinned.
- `fast_embedder_is_clone` (F11, D258): `fn clone_of<T: Clone>() {} clone_of::<FastEmbedder>();`
  (a compile-time check; the body does nothing at run time).
- The seven existing tests (`:224-313`) are unchanged.

### 2.4 Gate and commits

```bash
cargo test -p htui --all-features --lib concepts -- --test-threads=1
cargo test -p htui-store
cargo clippy -p htui -p htui-store --all-features --all-targets -- -D warnings
```

1. `test(mod-64): concepts::query and report_line` (red: both fns with `todo!()` bodies, the three
   tests).
2. `feat(mod-64): FastEmbedder is Clone; the CLI builds its query and report line in concepts`
   (green: the derive, both bodies, the CLI rewired).

---

## 3. T2: store requests, `ConceptsRuntime`, the index seam, the harness hook (D230–D232, D237, D238)

**First failing test**: `concepts_worker::tests::a_search_answers_the_fake_s_hits_at_its_address`.
Starts from T1 merged.

### 3.1 `crates/htui/Cargo.toml`

`:23` `testkit = []` becomes `testkit = ["htui-store/test-support"]`, with the comment line above it
extended: `# MOD-64 D230: and htui-store's `test-support`, so `concepts_worker::MemIndex` (over
`MemVectorStore`) compiles whenever `testkit` does.` Features do not enter `Cargo.lock`; it must not
move.

### 3.2 `crates/htui/src/concepts_worker.rs` (new)

`lib.rs:17`: `pub mod concepts_worker;` after `pub mod concepts;`. Nothing else in `lib.rs` changes;
`spawn_with` (`lib.rs:114`) keeps its signature.

```rust
//! The concepts index behind the TUI's search (MOD-64 D230-D232, D237, D238).
//!
//! `SearchConcepts` loads an embedding model and calls Qdrant; `IndexConcepts` reads every item of
//! the scope. Neither may run in the store worker's serial loop (`R-NF-3`): [`ConceptsRuntime`]
//! serves both on tasks of their own, the way `AgentRuntime::preview` serves a preview, and each
//! task sends its own reply. The index is an object-safe [`ConceptIndex`]: [`QdrantIndex`] in
//! production, `MemIndex` in tests.

use std::collections::HashMap;
use std::mem::Discriminant;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use htui_core::model::Scope;
use htui_store::Backend;
use htui_store::embed::FastEmbedder;
use htui_store::qdrant_settings::QdrantSettings;
use htui_store::vector::{Hit, QdrantStore, SearchQuery, VectorStore as _};
use htui_store::vector_sync::{Indexer, SyncReport};
use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinHandle;

use crate::store_worker::{Origin, ReplyEnvelope, RequestEnvelope, StoreReply, StoreRequest};

/// What both requests answer when nothing serves them: the test harness without a runtime (D231).
pub const NOT_AVAILABLE: &str = "concepts search is not available";

/// The two `StoreRequest::name`s this module serves, in that order (D241).
pub const REQUEST_NAMES: [&str; 2] = ["search_concepts", "index_concepts"];

/// One concepts answer, its error inside (D232, blueprint D242): a Qdrant failure is the overlay's
/// to show and never becomes `StoreReply::Failed`, which the shell also puts on the status line.
#[derive(Debug, Clone, PartialEq)]
pub enum ConceptsReply {
    /// Answer to `SearchConcepts`. `query` is the request's own, so a list is matched to the
    /// search it answers.
    Hits {
        /// The search this answers.
        query: SearchQuery,
        /// The hits, best first, or why there are none.
        outcome: Result<Vec<Hit>, String>,
    },
    /// Answer to `IndexConcepts`: what the run did, or why it stopped.
    Indexed(Result<SyncReport, String>),
}

/// The index seam (D230): object-safe, where `VectorStore` (`async fn`s) is not. Errors are display
/// strings: they are only ever shown.
pub trait ConceptIndex: Send + Sync {
    /// Hybrid search, scoped to `query.projects`.
    fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>>;
    /// `Indexer::sync` of every project of `scope`, reading through `backend`.
    fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>>;
}
```

**`QdrantIndex`** (production):

```rust
/// The production index (D238): settings re-read from the keyring and Qdrant re-connected per
/// request, so a URL changed in Settings > Qdrant applies to the next search; the embedding model
/// loaded once, on the blocking pool, and shared.
#[derive(Debug, Default)]
pub struct QdrantIndex {
    /// Filled by the first load that succeeds; a failed load leaves it empty (D238).
    embedder: Arc<OnceCell<FastEmbedder>>,
}

impl QdrantIndex {
    /// An index with no model loaded yet.
    #[must_use]
    pub fn new() -> Self;
    /// The model, loading it on first use (F9, blueprint D245).
    async fn embedder(&self) -> Result<FastEmbedder, String>;
    /// Qdrant at `settings`, over the shared model.
    async fn connect(&self, settings: &QdrantSettings) -> Result<QdrantStore<FastEmbedder>, String>;
}

/// The keyring's URL and key, read on the blocking pool (`QdrantSnapshot::fetch`'s rule,
/// `qdrant_settings_info.rs:25-31`), then `concepts::settings_from`.
async fn qdrant_settings() -> Result<QdrantSettings, String>;
```

Bodies:
- `qdrant_settings`: `tokio::task::spawn_blocking(|| Ok::<_, StoreError>((secret::get_qdrant_url()?,
  secret::get_qdrant_api_key()?)))`, `.await` mapped `|e| format!("the keyring read stopped: {e}")`,
  the inner `StoreError` mapped `to_string()`, then `crate::concepts::settings_from(url, key)` mapped
  `|e| format!("{e:#}")` (anyhow's chain). With no URL this is exactly `no Qdrant URL is stored; set
  one in Settings > Qdrant` (`concepts.rs:52-56`). Import `htui_core::store::StoreError` and
  `htui_store::secret`.
- `embedder` (D245):
  ```rust
  let cell = Arc::clone(&self.embedder);
  // Its own task: a superseded search that is aborted mid-load must not cancel the load, or the
  // next search starts a second download (F9). A concurrent search waits on this init.
  tokio::spawn(async move {
      cell.get_or_try_init(|| async {
          tokio::task::spawn_blocking(FastEmbedder::new)
              .await
              .map_err(|e| format!("the embedding model's loader stopped: {e}"))?
              .map_err(|e| e.to_string())
      })
      .await
      .cloned()
  })
  .await
  .map_err(|e| format!("the embedding model's loader stopped: {e}"))?
  ```
- `connect`: `let embedder = self.embedder().await?; QdrantStore::connect(settings,
  embedder).await.map_err(|e| format!("cannot reach Qdrant: {e}"))` (the CLI's context,
  `concepts.rs:84`).
- `impl ConceptIndex for QdrantIndex`: `search` = `Box::pin(async move { let settings =
  qdrant_settings().await?; let store = self.connect(&settings).await?;
  store.search(&query).await.map_err(|e| e.to_string()) })`; `sync` the same with
  `Indexer::sync(&backend, &scope, &store)`. **Order matters**: settings before the model, so a box
  with no URL fails in microseconds without downloading anything (the keyring test relies on it).

**`MemIndex`** (tests):

```rust
/// A [`ConceptIndex`] over `MemVectorStore` (D230): ranks by shared terms, needs no model. It can be
/// told to fail every call, and to wait before answering, for the error and supersede cases.
#[cfg(any(test, feature = "testkit"))]
#[derive(Debug, Default)]
pub struct MemIndex {
    store: htui_store::vector::MemVectorStore,
    failure: Option<String>,
    delay: Option<Duration>,
}

#[cfg(any(test, feature = "testkit"))]
impl MemIndex {
    /// An empty index.
    #[must_use]
    pub fn new() -> Self;
    /// Every call answers `Err(message)` (after the delay, if any).
    #[must_use]
    pub fn failing(self, message: impl Into<String>) -> Self;
    /// Every call sleeps `by` first (`tokio::time::sleep`, so a paused clock advances it).
    #[must_use]
    pub fn delayed(self, by: Duration) -> Self;
    /// The fake store, for assertions.
    #[must_use]
    pub fn store(&self) -> &htui_store::vector::MemVectorStore;
    /// Indexes `scope` through `backend` directly, as a test's setup.
    ///
    /// # Panics
    /// If the sync fails, which `MemVectorStore` never does over a memory backend.
    pub async fn seed(&self, backend: &Backend, scope: &Scope) -> SyncReport;
}
```

`impl ConceptIndex for MemIndex`: `search` = `Box::pin(async move { self.pause().await; if let
Some(message) = &self.failure { return Err(message.clone()); } self.store.search(&query).await
.map_err(|e| e.to_string()) })`; `sync` the same around `Indexer::sync(&backend, &scope,
&self.store)`. Private `async fn pause(&self)`. (`MemVectorStore` holds a `std::sync::Mutex` but never
across an await, `vector.rs:780-891`; the plan's probe K2 compiled `tokio::spawn` of a sync over it.)

**`ConceptsRuntime`**:

```rust
/// What [`ConceptsRuntime::serve`] decided (the `RunServed` shape, blueprint D243).
#[derive(Debug)]
pub enum ConceptsServed {
    /// Answer with this reply, now.
    Reply(StoreReply),
    /// A task of the runtime answers the request, exactly once.
    Deferred,
}

/// Serves `SearchConcepts` and `IndexConcepts` on tasks of their own, inside the store worker's
/// loop beside `AgentRuntime` and `RunRuntime` (D231). One task per `(origin, request kind)`: a
/// newer request of the same kind from the same view aborts the older one, whose answer the shell
/// would drop anyway. A search never aborts an index run: they are different kinds.
pub struct ConceptsRuntime {
    index: Arc<dyn ConceptIndex>,
    tasks: HashMap<(Origin, Discriminant<StoreRequest>), JoinHandle<()>>,
}

impl core::fmt::Debug for ConceptsRuntime { /* "ConceptsRuntime" { tasks: self.tasks.len() }, finish_non_exhaustive */ }

impl ConceptsRuntime {
    /// A runtime over `index`.
    #[must_use]
    pub fn new(index: Arc<dyn ConceptIndex>) -> Self;
    /// Over [`QdrantIndex`]: what `store_worker::spawn_with_runtimes` builds (D231).
    #[must_use]
    pub fn production() -> Self;
    /// Spawns the request's task and answers `Deferred`, or answers now. Awaits nothing.
    pub fn serve(
        &mut self,
        backend: &Backend,
        replies: &mpsc::UnboundedSender<ReplyEnvelope>,
        envelope: &RequestEnvelope,
    ) -> ConceptsServed;
    /// Harness only (`RunRuntime::settle`'s shape): awaits every task, each within `limit`, and
    /// answers how many did not finish (they are aborted).
    pub async fn settle(&mut self, limit: Duration) -> usize;
    /// The UI is gone: every task is aborted. An index run stopped half-way is safe: the next run
    /// rebuilds what it did not reach (`vector_sync.rs:1-13`).
    pub fn shutdown(&mut self);
}
```

`serve` body, in order:
1. `self.tasks.retain(|_, task| !task.is_finished());`
2. `let key = (envelope.origin.clone(), std::mem::discriminant(&envelope.request));`,
   `let (seq, origin) = (envelope.seq, envelope.origin.clone());`, `let index =
   Arc::clone(&self.index);`, `let replies = replies.clone();`.
3. `match &envelope.request`:
   - `SearchConcepts(query)`: if `query.projects.is_empty() || query.limit == 0` → return
     `ConceptsServed::Reply(StoreReply::Concepts(Box::new(ConceptsReply::Hits { query:
     query.clone(), outcome: Ok(Vec::new()) })))` without touching the index (D233; the index says
     `[]` too, `vector.rs:728-731`). Else spawn: `let outcome = index.search(query.clone()).await;
     let _ = replies.send(ReplyEnvelope { seq, origin, reply: Concepts(Box::new(Hits { query,
     outcome })) });`.
   - `IndexConcepts { scope }`: if `scope.project_ids.is_empty()` → `Reply(Concepts(Indexed(Ok(
     SyncReport::default()))))`. Else spawn over `backend.clone()` and `scope.clone()` (`Backend:
     Clone`, `backend.rs:50-70`; a clone cannot swap the worker's backend, the preview's rule,
     `agent_worker.rs:1345-1347`), sending `Indexed(outcome)`.
   - `other` → `Reply(StoreReply::Failed { request: other.name(), message: "not a concepts
     request".to_owned() })` (the two runtimes' rule, `agent_worker.rs:1117-1120`,
     `run_worker.rs:1621-1624`; unreachable from the loop).
4. `if let Some(superseded) = self.tasks.insert(key, handle) { superseded.abort(); }` then
   `ConceptsServed::Deferred`.

`settle`: `for (_, task) in self.tasks.drain() { let abort = task.abort_handle(); if
tokio::time::timeout(limit, task).await.is_err() { abort.abort(); stuck += 1; } }`. An already
aborted or finished handle resolves at once. `shutdown`: `for (_, task) in self.tasks.drain() {
task.abort(); }`.

### 3.3 `crates/htui/src/store_worker.rs`

- **Imports** (`:33-53`): `use htui_store::vector::SearchQuery;` and
  `use crate::concepts_worker::{self, ConceptsReply, ConceptsRuntime, ConceptsServed};`.
- **`StoreRequest`**, after `ClearQdrantSettings` (`:641-642`):
  ```rust
  /// MOD-64 D231: a concepts search, served by `concepts_worker::ConceptsRuntime` on a task of its
  /// own (`R-NF-3`). Answered with [`StoreReply::Concepts`], never `Failed` (D232).
  SearchConcepts(SearchQuery),
  /// MOD-64 D237: `Indexer::sync` over `scope`'s projects, on the runtime's task. Answered with
  /// [`StoreReply::Concepts`].
  IndexConcepts {
      /// The workspace and the projects to index.
      scope: Scope,
  },
  ```
  Neither carries a secret (the rule at `:89-96`).
- **`name()`**, after `Self::ClearQdrantSettings => "clear_qdrant_settings",` (`:832`), under
  `// The two of concepts_worker::REQUEST_NAMES, in that order (MOD-64 D241).`:
  `Self::SearchConcepts(_) => "search_concepts",` and `Self::IndexConcepts { .. } => "index_concepts",`.
- **`StoreReply`**, after `Qdrant(..)` (`:1032-1033`):
  ```rust
  /// Answer to [`StoreRequest::SearchConcepts`] and [`StoreRequest::IndexConcepts`] (MOD-64 D232):
  /// the outcome carries its own error, so a Qdrant failure stays in the search overlay and never
  /// reaches the status line the way a [`StoreReply::Failed`] does.
  Concepts(Box<ConceptsReply>),
  ```
- **`try_serve`**, after the Qdrant arm (`:1384-1390`), two arms (no wildcard exists, `:1251-1408`):
  ```rust
  // MOD-64 D231: the loop serves both through the concepts runtime; one that reaches here belongs to
  // a caller with none (the harness default), and is answered in the overlay's own reply (D232).
  StoreRequest::SearchConcepts(query) => StoreReply::Concepts(Box::new(ConceptsReply::Hits {
      query: query.clone(),
      outcome: Err(concepts_worker::NOT_AVAILABLE.to_owned()),
  })),
  StoreRequest::IndexConcepts { .. } => StoreReply::Concepts(Box::new(ConceptsReply::Indexed(
      Err(concepts_worker::NOT_AVAILABLE.to_owned()),
  ))),
  ```
- **`spawn_with_runtimes`** (`:1526-2027`), signature unchanged (D231; its test caller
  `run_worker.rs:2790` does not move):
  - After `let mut run_events = runs.take_events();` (`:1576`): `// MOD-64 D231: the concepts
    runtime, built here so no caller's signature moves.` `let mut concepts =
    ConceptsRuntime::production();`
  - A new arm between the run arm (`:1886-1896`) and `other => match try_serve(..)` (`:1897`):
    ```rust
    // MOD-64 D231: a search loads a model and calls Qdrant, an index run reads every item: both
    // are tasks of the concepts runtime, and `Deferred => continue` is the whole of `R-NF-3`.
    StoreRequest::SearchConcepts(_) | StoreRequest::IndexConcepts { .. } => {
        match concepts.serve(&backend, &tx, &envelope) {
            ConceptsServed::Reply(reply) => reply,
            ConceptsServed::Deferred => continue,
        }
    }
    ```
  - After the `tokio::join!(..)` (`:2015-2018`): `concepts.shutdown();`.

### 3.4 `crates/htui/src/testkit.rs`

- Import: `use crate::concepts_worker::{ConceptsRuntime, ConceptsServed};`.
- `Harness` field after `run_events` (`:67-68`): `/// The concepts runtime, when a test installed one
  (MOD-64 D231). Without it both requests answer `concepts_worker::NOT_AVAILABLE`.`
  `concepts: Option<ConceptsRuntime>,`; `over_backend` (`:124-136`) sets `concepts: None`.
- After `with_run_runtime` (`:160-164`):
  ```rust
  /// Installs the runtime the concepts search and index requests are served by (MOD-64 D231), as
  /// [`Harness::with_agent_runtime`] installs the chat one. [`Harness::drive`] then awaits its
  /// tasks every round, so a render never photographs a search in flight by accident.
  /// [`Harness::settle`] stays runtime-free: it answers both requests `NOT_AVAILABLE`.
  #[must_use]
  pub fn with_concepts_runtime(mut self, runtime: ConceptsRuntime) -> Self
  ```
- `drive`: a new arm before `(request, _) => store_worker::serve(..)` (`:333`):
  ```rust
  // MOD-64 D231: through the concepts runtime when a test installed one; without one
  // `store_worker::serve` answers them `NOT_AVAILABLE` (D244).
  (StoreRequest::SearchConcepts(_) | StoreRequest::IndexConcepts { .. }, _) => {
      match self.concepts.as_mut() {
          Some(concepts) => match concepts.serve(&self.backend, &self.replies.0, &envelope) {
              ConceptsServed::Reply(reply) => reply,
              ConceptsServed::Deferred => continue,
          },
          None => store_worker::serve(&self.backend, &envelope.request).await,
      }
  }
  ```
  After the request loop, before `if let Some(runs) = self.runs.as_mut()` (`:344`):
  ```rust
  // MOD-64 D243: every search and index task this round spawned has answered before the replies
  // below are read.
  if let Some(concepts) = self.concepts.as_mut() {
      let stuck = concepts.settle(CHAT_END).await;
      assert_eq!(stuck, 0, "{stuck} concepts task(s) did not end within {CHAT_END:?}");
  }
  ```
  `settle` (`:484-516`) is not touched.

### 3.5 Tests (written first)

In `concepts_worker.rs` `mod tests` (`#[tokio::test]`; a helper builds `(Backend::memory(MemStore::
demo()), the Platform scope)` the way `requirements/mod.rs:1225-1240` finds `platform`, a helper
`serve_one(runtime, envelope) -> Vec<ReplyEnvelope>` that serves, `settle`s and drains an unbounded
receiver):

| Test | Asserts |
|---|---|
| `a_search_answers_the_fake_s_hits_at_its_address` | Seeded `MemIndex`; `SearchConcepts(concepts::query(<words of FEAT-1's title>, platform ids, false, 10))` at `seq 7`, `Origin::Overlay(OverlayId("probe"))` → `Deferred`; one reply at seq 7 / that origin; `Concepts(Hits { query, outcome: Ok(hits) })` with `query` equal to the sent one and `hits` equal to `index.store().search(&query)` directly (same order). |
| `decisions_narrow_the_hits_to_closed_items` | Same text with `decisions = true`: every hit has `resolution` in `DECISION_RESOLUTIONS` and none is a requirement. |
| `a_search_with_no_projects_answers_empty_without_the_index` | `MemIndex::new().failing("boom")`; empty `projects` → `ConceptsServed::Reply(Concepts(Hits { outcome: Ok(vec![]) , .. }))` returned directly (the failing index was never called). |
| `a_second_search_from_the_same_origin_aborts_the_first` | `#[tokio::test(start_paused = true)]`; seeded index `.delayed(1 s)`; serve seq 1 then seq 2 from one origin; `settle(10 s)` → 0 stuck; exactly one reply, seq 2. |
| `a_search_does_not_abort_an_index_run` | `.delayed(1 s)`; `IndexConcepts` seq 1 then `SearchConcepts` seq 2, one origin; replies at both seqs. |
| `an_index_run_reports_its_counts` | Empty `MemIndex`, `IndexConcepts { scope: platform }` → `Indexed(Ok(report))` with `items_rebuilt > 0` and `points_upserted == index.store().points().len()`; a second run → `items_rebuilt == 0`. |
| `index_and_search_errors_are_concepts_replies_never_failed` | `failing("qdrant: query: refused")`: the search answers `Hits { outcome: Err(m) }`, the index run `Indexed(Err(m))`, `m` the message; neither reply is `StoreReply::Failed`. |
| `without_a_runtime_both_requests_answer_not_available` | `store_worker::serve(&backend, ..)` for each → `Concepts(Hits { outcome: Err(NOT_AVAILABLE) })` / `Concepts(Indexed(Err(NOT_AVAILABLE)))`. |
| `request_names_match_the_name_arms` | `REQUEST_NAMES[0] == SearchConcepts(..).name()`, `[1] == IndexConcepts { .. }.name()`. |
| `qdrant_index_without_a_stored_url_names_where_to_set_it` | First statement `let _keyring = htui_store::testkit::mock_keyring().await;`; `QdrantIndex::new().search(q).await` and `.sync(backend, scope).await` are both `Err` containing `Settings > Qdrant`; the embedder cell is still empty (`self.embedder.get().is_none()`, via a `#[cfg(test)]` accessor or a test in the same module). Run under `--test-threads=1`. |
| `shutdown_aborts_every_task` | `.delayed(1 h)`, serve one of each kind, `shutdown()`, `settle(1 ms)` → 0 (nothing left), no reply. |

In `store_worker.rs` `mod tests` (the `detached` helper at `:2227-2236`):
`the_loop_serves_a_search_off_the_loop_and_never_as_failed`: `mock_keyring()` first; `spawn` over
`Started::detached(demo())`; send `SearchConcepts(q)` with a non-empty project list and then
`BoxInfo`; both answer, the search as `Concepts(Hits { outcome: Err(e) })` with `e` containing
`Settings > Qdrant`. (The production runtime reads only the fake keyring, which is empty, so no model
loads and no network is touched.)

The harness path (`with_concepts_runtime`, `drive`, the `NOT_AVAILABLE` fallback) is exercised end to
end by T4 (§5.8); T2 does not add an overlay to test it with.

### 3.6 Build coupling

- Needs T1's `FastEmbedder: Clone` (`.cloned()` in `embedder`). Not `query`/`report_line` (T4's).
- `observe_reply` has a `_` arm (`update.rs:238`) and no view matches `StoreReply` exhaustively, so
  the new variant touches no view.
- Adding `SearchQuery` to `StoreRequest` keeps `StoreRequest: Debug + Clone`.

### 3.7 Gate and commits

```bash
cargo test -p htui --all-features --lib concepts_worker -- --test-threads=1
cargo test -p htui --all-features --lib store_worker -- --test-threads=1
cargo clippy -p htui --all-features --all-targets -- -D warnings
git diff --exit-code Cargo.lock
```

1. `test(mod-64): the concepts runtime serves search and index off the loop` (red: every type and
   signature of §3.2–§3.4 with `todo!()` bodies where logic goes, the variants and arms wired, the
   tests).
2. `feat(mod-64): ConceptsRuntime, SearchConcepts/IndexConcepts and the harness hook` (green).

---

## 4. T3: reveal (action, shell routing, tab hooks) and the freshness fix (D235, D240)

**First failing test**: `app::update::tests::a_reply_to_a_closed_overlay_never_reaches_the_next_one`.

### 4.1 `crates/htui/src/app/action.rs`

- Import `htui_core::model::{ItemId, RequirementId, RunId, StepId, WorkspaceSummary}` (`:6`).
- `Action`, after `Promote` (`:43-50`):
  ```rust
  /// Select an entity in the tab that shows it (MOD-64 D235). Emitted by the concepts search; the
  /// shell focuses the tab registered for the target's kind (`App::reveal_tabs`) and hands it the
  /// target through `Tab::reveal`, so the overlay never names a tab.
  Reveal(RevealTarget),
  ```
- After `OverlayAction` (`:84-93`):
  ```rust
  /// What `Action::Reveal` selects (MOD-64 D235). The key rides along for the sentence a tab says
  /// when the row is not in this workspace (blueprint D248).
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum RevealTarget {
      /// An item; a document hit reveals its owner item.
      Item {
          /// The item.
          id: ItemId,
          /// Its key, e.g. `FEAT-1`.
          key: String,
      },
      /// A requirement.
      Requirement {
          /// The requirement.
          id: RequirementId,
          /// Its key, e.g. `R-STO-8`.
          key: String,
      },
  }

  impl RevealTarget {
      /// Which registration routes it.
      #[must_use]
      pub const fn kind(&self) -> RevealKind;
      /// The target's key.
      #[must_use]
      pub fn key(&self) -> &str;
  }

  /// Which kind of entity a tab reveals: the key of `App::reveal_tabs`.
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
  pub enum RevealKind {
      /// Items (the Backlog).
      Item,
      /// Requirements (the Requirements tab).
      Requirement,
  }
  ```
- `app/mod.rs:6`: `pub use action::{Action, Handled, OverlayAction, RevealKind, RevealTarget, TabAction};`.

### 4.2 `crates/htui/src/app/state.rs` and the D240 fix

- `App` field after `replay_tab` (`:176-183`):
  ```rust
  /// Which tab reveals which kind of entity (MOD-64 D235). Set by
  /// [`register_all`](crate::app::register_all), ids for the reason `replay_tab` is one: the shell
  /// names no concrete view. A kind with no entry reveals nothing.
  pub reveal_tabs: Vec<(RevealKind, TabId)>,
  ```
  `App::new` (`:209-236`): `reveal_tabs: Vec::new(),` after `replay_tab: None,`. Import
  `crate::app::action::RevealKind` (`:14`).
- After `is_fresh` (`:318-327`):
  ```rust
  /// Forgets every staleness entry of `origin` (MOD-64 D240), so no reply to a request made before
  /// this can pass [`App::is_fresh`] again.
  pub(super) fn forget(&mut self, origin: &Origin) {
      self.latest.retain(|(known, _), _| known != origin);
  }
  ```
- `update.rs`, `update_overlay` (`:77-89`): `OverlayAction::Close => self.close_top_overlay(),`,
  `OverlayAction::CloseAll => self.close_every_overlay(),`. `set_scope` (`:113-127`):
  `self.overlays.clear();` (`:122`) becomes `self.close_every_overlay();`. New, after
  `update_overlay`:
  ```rust
  /// Pops the top overlay and forgets its requests (MOD-64 D240): `latest` is keyed by origin, and
  /// a later overlay under the same id would otherwise take a reply to one of this one's.
  fn close_top_overlay(&mut self) {
      let Some(id) = self.overlays.top().map(Overlay::id) else { return };
      self.overlays.pop();
      self.forget_overlay(id);
  }

  /// Closes every overlay and forgets their requests (D240): `CloseAll` and a scope change.
  fn close_every_overlay(&mut self) {
      let ids: Vec<OverlayId> = self.overlays.iter().map(Overlay::id).collect();
      self.overlays.clear();
      for id in ids {
          self.forget_overlay(id);
      }
  }

  /// [`App::forget`] for `Origin::Overlay(id)`, unless an overlay under that id is still open.
  fn forget_overlay(&mut self, id: OverlayId) {
      if !self.overlays.iter().any(|open| open.id() == id) {
          self.forget(&Origin::Overlay(id));
      }
  }
  ```
  Import `crate::ui::overlay::{Overlay, OverlayId}` at the top of `update.rs`.

### 4.3 `App::reveal` (`update.rs`) and `Tab::reveal` (`registry.rs`)

`update()` (`:22-44`): `Action::Reveal(target) => self.reveal(&target),` after the `Promote` arm.
After `promote` (`:144-161`):

```rust
/// Selects an entity in the tab registered for its kind (MOD-64 D235): focus that tab — which
/// re-sends its `wants_requests` if it was not the active one — then hand it the target with a
/// `Ctx` of its own, as `finish_external_edit` does, and drain what it emitted. A kind nothing is
/// registered for is a no-op (T3's test), logged.
fn reveal(&mut self, target: &RevealTarget) {
    let kind = target.kind();
    let Some(tab) = self.reveal_tabs.iter().find(|(k, _)| *k == kind).map(|(_, tab)| *tab) else {
        tracing::debug!(?kind, "no tab reveals this kind");
        return;
    };
    self.update_tab(TabAction::Focus(tab));
    let origin = Origin::Tab(tab);
    {
        let Self { scope, projects, top_bar, keymap, theme, emit, tabs, .. } = self;
        let Some(view) = tabs.by_id_mut(tab) else { return };
        let mut ctx = Ctx::new(scope, projects, top_bar, keymap, theme, origin.clone(), emit);
        if !view.reveal(target, &mut ctx) {
            tracing::debug!(%tab, "the tab does not reveal this target");
        }
    }
    self.drain(&origin);
}
```

`registry.rs`, after `on_external_edit` (`:61-63`), import `crate::app::RevealTarget`:

```rust
/// Selects `target` (MOD-64 D235): unfold or clear what hides it, move the cursor, read its
/// detail; or, when this tab has not loaded it, keep it until the next list reply. `false` when
/// this tab shows no such thing — every tab but Backlog and Requirements, which is what the default
/// says. The trait's third default, after `focus_section` and `on_external_edit`.
fn reveal(&mut self, _target: &RevealTarget, _ctx: &mut Ctx<'_>) -> bool {
    false
}
```

Also in `registry.rs`: `/// What a tab says when a reveal arrives over a field the user is typing in (MOD-64 D252).`
`pub const CLOSE_THE_FIELD_FIRST: &str = "close the open field (Esc) first, then search again";` *(Amended at review, MOD-64 review 7: was "then open the hit again", but the overlay has closed and a reopened one is empty.)*

### 4.4 `crates/htui/src/ui/tabs/backlog/mod.rs`

- Field after `detail` (`:47`): `/// A reveal waiting for the next `Items` reply (MOD-64 D235), with the
  key its miss is reported by.` `pending_reveal: Option<(ItemId, String)>,`; `new` (`:66-71`) and the
  test literal (`:351-356`, F13) set `None`. `on_scope_change` (`:204-209`) sets it to `None`.
- Import `crate::app::{Action, Ctx, Handled, RevealTarget}` and
  `crate::ui::tabs::registry::CLOSE_THE_FIELD_FIRST`.
- Free fn after `panes` (`:275-281`): `/// The Backlog's sentence for a reveal of an item this workspace
  does not hold (MOD-64 D235, F5).` `#[must_use] pub fn not_in_this_backlog(key: &str) -> String {
  format!("{key} is not in this workspace's backlog") }`.
- Private helper in `impl BacklogTab`, after `reselect` (`:185-196`):
  ```rust
  /// Unfolds `project` and moves the cursor to `id`, reading its detail (D235).
  fn select_item(&mut self, id: ItemId, project: ProjectId, ctx: &Ctx<'_>) {
      self.folded.retain(|folded| *folded != project);
      self.go(Some(Selection::Item(id)), ctx);
  }
  ```
- `impl Tab`, after `render` (`:262-272`):
  ```rust
  fn reveal(&mut self, target: &RevealTarget, ctx: &mut Ctx<'_>) -> bool {
      let RevealTarget::Item { id, key } = target else { return false };
      // A half-typed note or reject reason would be lost by the move (`go` resets the sub-tabs).
      if self.detail.captures_input() {
          ctx.emit(Action::Error(CLOSE_THE_FIELD_FIRST.to_owned()));
          return true;
      }
      match self.items.iter().find(|item| item.id == *id).map(|item| item.project_id) {
          Some(project) => {
              self.pending_reveal = None;
              self.select_item(*id, project, ctx);
          }
          // Not loaded, or not in the rows read so far: re-read and decide on arrival (D251).
          None => {
              self.pending_reveal = Some((*id, key.clone()));
              ctx.request(StoreRequest::Items { scope: ctx.scope.clone(), filter: ItemFilter::default() });
          }
      }
      true
  }
  ```
- `on_reply` (`:238-247`), in the `Items` branch between the `folded.retain` and `reselect`:
  ```rust
  if let Some((id, key)) = self.pending_reveal.take() {
      match self.items.iter().find(|item| item.id == id).map(|item| item.project_id) {
          Some(project) => self.select_item(id, project, ctx),
          None => ctx.emit(Action::Error(not_in_this_backlog(&key))),
      }
  }
  ```
  `reselect` then finds the cursor visible and returns (`:185-196`).

The second `Items` request supersedes the one `activate_tab` sent under the same
`(Tab(backlog), Items)` key; only the newer reply lands (`state.rs:318-327`).

### 4.5 `crates/htui/src/ui/tabs/requirements/mod.rs`

- Rename the private `reveal` (`:800-819`) to `unhide`; doc unchanged (F6).
- `select_row`, after `land` (`:731-799`):
  ```rust
  /// Makes `row` visible, puts the cursor on it and re-reads its detail: `land`'s tail, shared with
  /// a reveal (MOD-64 D235). The detail is re-read even when `row` was already selected — `land`
  /// needs that, the row having just been written.
  fn select_row(&mut self, row: Row, ctx: &Ctx<'_>) {
      self.unhide(row, ctx);
      if self.selected != Some(row) {
          self.detail = None;
          self.scroll.reset();
      }
      self.selected = Some(row);
      self.detail_error = None;
      if let Row::Requirement(id) = row {
          ctx.request(StoreRequest::RequirementDetail(id));
      }
  }
  ```
  `land` keeps `:784-787` (`busy`, `sent`, `mode`, `notice`) then `self.select_row(row, ctx); true`.
  Behaviour is byte-identical; the existing land tests (`tests/requirements.rs` `n_mints_…`,
  `a_stale_amend_…`, the `requirements__requirements_{minted,amended,withdrawn,new_area}` snapshots)
  pin it.
- Field after `notice` (`:187`): `/// A reveal waiting for the next `Requirements` reply (MOD-64 D235).`
  `pending_reveal: Option<(RequirementId, String)>,`. `Default` covers it; `on_scope_change` resets
  the whole tab (`:1063-1065`).
- Free fn after `requirement_changed_elsewhere` (`:124-131`): `#[must_use] pub fn
  not_in_these_requirements(key: &str) -> String { format!("{key} is not in this workspace's
  requirements") }`.
- `impl Tab`, after `render`:
  ```rust
  fn reveal(&mut self, target: &RevealTarget, ctx: &mut Ctx<'_>) -> bool {
      let RevealTarget::Requirement { id, key } = target else { return false };
      match self.mode {
          Mode::Browse => {}
          // The typed filter is kept as applied; `unhide` drops it if it hides the row.
          Mode::Filter { .. } => self.mode = Mode::Browse,
          // An open form keeps its text: the reveal says why it did not move (D252).
          Mode::NewArea(_) | Mode::Requirement(_) | Mode::Withdraw(_) => {
              self.notice = Some(Notice::Error(CLOSE_THE_FIELD_FIRST.to_owned()));
              return true;
          }
      }
      if self.snapshot.as_ref().is_some_and(|snapshot| snapshot.requirement(*id).is_some()) {
          self.pending_reveal = None;
          self.select_row(Row::Requirement(*id), ctx);
      } else {
          self.pending_reveal = Some((*id, key.clone()));
          ctx.request(StoreRequest::Requirements(ctx.scope.clone()));
      }
      true
  }
  ```
- `on_reply`, `Requirements` branch (`:1089-1107`): after the `verifying` block and before
  `self.reselect(ctx)`:
  ```rust
  if let Some((id, key)) = self.pending_reveal.take() {
      if self.snapshot.as_ref().is_some_and(|snapshot| snapshot.requirement(id).is_some()) {
          self.select_row(Row::Requirement(id), ctx);
      } else {
          self.notice = Some(Notice::Error(not_in_these_requirements(&key)));
      }
  }
  ```
  A `busy` reply that `land`s returns before this (`:1095-1098`); the pending reveal then waits for
  the next read, which is correct (the write's own row wins that frame). The `Failed { request ==
  READ_NAME }` arm (`:1127-1142`) sets `self.pending_reveal = None;` first (D251: a refused read must
  not leave a jump armed for a later one).

### 4.6 `crates/htui/src/app/mod.rs`

After `app.replay_tab = Some(ChatTab::ID);` (`:88`):
```rust
// MOD-64 D235: which tab selects a revealed item or requirement. Ids, not views, for the reason
// `replay_tab` is one.
app.reveal_tabs = vec![
    (RevealKind::Item, BacklogTab::ID),
    (RevealKind::Requirement, RequirementsTab::ID),
];
```
The doc list of `register_all` (`:21-45`) gains `6. The Backlog and Requirements tabs are named as
the tabs that reveal items and requirements (MOD-64 D235).`

### 4.7 Tests (written first)

`app/update.rs` `mod tests` (reuses `shell()`, `Recorder`, `Popup`; F8, D247):
- A test overlay `Asking { seen: Rc<RefCell<Vec<StoreReply>>> }`, `ID = OverlayId("asking")`,
  `wants_requests` empty, `on_key` → `ctx.request(StoreRequest::Workspaces)`, `on_reply` pushes.
  Registered with `app.overlay_factories.register(Asking::ID, move || …)`.
- `a_reply_to_a_closed_overlay_never_reaches_the_next_one`: open, key `s` → take the `Workspaces`
  seq from `rx`, `Overlay(Close)`, open again, deliver `Reply { seq, Origin::Overlay(Asking::ID),
  Workspaces(vec![]) }` → `seen` is empty. (Red before §4.2: the entry survives and the reopened
  overlay receives it.)
- `close_all_and_a_scope_change_forget_an_overlays_requests_too`: the same through `CloseAll`, then
  through `SetScope`.
- `closing_the_top_overlay_keeps_the_one_below_fresh`: `Asking` below, `Popup` on top; `Asking`'s
  request (issued before `Popup` opened) answers after `Close` pops `Popup` → delivered.
- A `Revealer` tab (`TabId("revealer")`) whose `reveal` records the target, requests `Items`, answers
  `true`. `reveal_focuses_the_registered_tab_and_hands_it_the_target`: `reveal_tabs = vec![(Item,
  revealer)]`, a second tab active; `update(Reveal(Item { .. }))` → `tabs.active_id() ==
  Some(revealer)`, the target recorded, and an `Items` envelope with `Origin::Tab(revealer)` on `rx`.
- `a_reveal_kind_no_tab_is_registered_for_does_nothing`: `Requirement` target, no entry → active
  tab unchanged, `status == None`, nothing recorded.

`crates/htui/tests/reveal.rs` (new, `#![cfg(feature = "testkit")]`). Helper `platform()`:
`Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()))`, `register_all`,
`drive_to_end`, keys `w` `j` `enter`, `drive_to_end`, assert `top_bar.workspace == "Platform"` (the
`tests/requirements.rs:37-50` walk; the agent runtime because `BacklogTab::go` sends a
`PromptPreview`, which `settle` would answer `Failed` onto the status line). `reveal(h, target)` =
`h.app().update(Action::Reveal(target)); h.drive_to_end().await`. Selection is read from the detail
pane's title, `format!("\u{250c} {key} ")` (`backlog__list_grouped.snap:7`,
`requirements__requirements_tree.snap:7`). Keys chosen unique in Platform: `TOOL-1`
(`ids::HTUI_TOOL_1`), `R-STO-1` (not `R-ENT-1`, which `reselect` lands on by itself).

| Test | Drive | Asserts |
|---|---|---|
| `revealing_a_loaded_item_focuses_the_backlog_and_selects_it` | key `3`, `drive_to_end`; reveal `Item { HTUI_TOOL_1, "TOOL-1" }` | active `BacklogTab::ID`; frame contains `┌ TOOL-1 ` and TOOL-1's title in the Body pane; `status == None` |
| `revealing_an_item_under_a_folded_project_unfolds_it` | on Backlog, `g` (htui header), `enter` (folds; frame shows `▸ htui`); reveal TOOL-1 | frame shows `▾ htui` and `┌ TOOL-1 ` |
| `revealing_before_the_backlog_has_loaded_selects_on_arrival` | after `w` `j` `enter` **without** a drive (the scope change cleared the rows and the `Items` read is still queued), reveal via `update` then `drive_to_end` | `┌ TOOL-1 ` |
| `revealing_an_unknown_item_says_so_and_keeps_the_cursor` | note the detail title; reveal `Item { ItemId::new(), "NOPE-1" }` | `status == Some(not_in_this_backlog("NOPE-1"))`; the title is unchanged |
| `revealing_a_requirement_focuses_the_requirements_tab_and_opens_it` | from Backlog; reveal `Requirement { REQ_STO_1, "R-STO-1" }` | active `RequirementsTab::ID`; `┌ R-STO-1 `; the detail body (not `select a requirement`) |
| `revealing_a_requirement_the_filter_hides_clears_the_filter` | `3`, `/`, type `priority`, `enter` (only R-ENT-2 matches, `tree.rs:352-360`; R-STO-1 hidden); reveal R-STO-1 | tree title is ` Requirements (3) ` without `· /`; `┌ R-STO-1 ` |
| `revealing_an_unknown_requirement_says_so_on_the_tab` | reveal `Requirement { RequirementId::new(), "R-NOPE-1" }` | frame's notice row contains `R-NOPE-1 is not in this workspace's requirements` |
| `a_reveal_over_an_open_form_keeps_the_form` | `3`, select an area, `n` (form open), type text; reveal R-STO-1 | notice `CLOSE_THE_FIELD_FIRST`; the typed text still in the form |

`ids::REQ_STO_1` is `R-STO-1` (used by `requirements/tree.rs:337`); `ids::HTUI_TOOL_1` is `htui`
`TOOL-1` (`fixtures.rs:239-240`).

### 4.8 Gate and commits

```bash
cargo test -p htui --all-features --lib app:: -- --test-threads=1
cargo test -p htui --all-features --test reveal --test requirements --test backlog --test integration -- --test-threads=1
cargo clippy -p htui --all-features --all-targets -- -D warnings
```

1. `test(mod-64): a popped overlay's requests are forgotten` (red: the `update.rs` D240 tests).
2. `fix(mod-64): closing an overlay forgets its freshness entries (D240)` (green: §4.2).
3. `refactor(mod-64): RequirementsTab::select_row is land's tail; reveal becomes unhide`
   (behaviour-neutral; requirements tests green before and after).
4. `test(mod-64): reveal an item or a requirement from the shell` (red: §4.1 types, `Tab::reveal`
   default, `App::reveal` and the two tab impls as `todo!()`, the `reveal_tabs` registration, the
   `update.rs` reveal tests, `tests/reveal.rs`).
5. `feat(mod-64): Action::Reveal selects in the Backlog and the Requirements tab (D235)` (green).

---

## 5. T4: the overlay, its registration and its tests (D233, D234, D236)

**First failing test**: `tests/concepts_search.rs::enter_searches_and_lists_hits_as_the_cli_prints_them`.
Starts from T2 and T3 merged.

### 5.1 Files

- New `crates/htui/src/ui/overlay/concepts_search.rs`.
- `ui/overlay/mod.rs`: `pub mod concepts_search;` (before `pub mod migration_prompt;`) and
  `pub use concepts_search::ConceptsSearch;`.
- `app/mod.rs` (`register_all`, after the `w` binding `:68-73`):
  ```rust
  // MOD-64 D236: the concepts search, global `Ctrl+F`. A chord, so no tab's letters and no text
  // field's input can take it (every text widget passes chords, blueprint F7).
  app.overlay_factories
      .register(ConceptsSearch::ID, || Box::new(ConceptsSearch::new()));
  app.keymap.bind(Binding {
      scope: KeyScope::Global,
      key: KeyChord::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
      action: Action::Overlay(OverlayAction::Open(ConceptsSearch::ID)),
      help: "find",
  });
  ```
  Import `ConceptsSearch` in `:12`; the doc list gains `7. …Ctrl+F opens the concepts search
  (MOD-64 D236)`.
- `ui/tabs/chat/mod.rs` (F7, D256): `:43` imports `KeyModifiers`; in `on_key` after
  `self.refusal = None;` (`:463`):
  ```rust
  // A chord is the shell's (`Ctrl+F`, `Ctrl+C`), never composer text: every other text widget
  // passes it the same way (`TextField::on_key`, MOD-64 F7).
  if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
      return Handled::Pass;
  }
  ```
- New `crates/htui/tests/concepts_search.rs`; 5 new snapshots; 27 moved (§5.9).

### 5.2 State

```rust
/// The concepts search (MOD-64): a query field, a project scope, a decisions toggle and the last
/// hits, searched off the UI thread through `StoreRequest::SearchConcepts`.
#[derive(Debug, Default)]
pub struct ConceptsSearch {
    /// The query as typed.
    field: TextField,
    /// `None`: every project of the workspace; `Some`: the one `Ctrl+P` cycled to (D233).
    project: Option<ProjectId>,
    /// `Ctrl+D`: decisions only.
    decisions: bool,
    /// The last search sent, in flight or answered: what tells `Enter`-searches from `Enter`-opens
    /// (D234, blueprint D255). `None` before the first and after a failed one, so `Enter` retries.
    sent: Option<SearchQuery>,
    /// A search is in flight.
    searching: bool,
    /// This overlay has had a search answered: `searching…` rather than the model line (F10).
    answered: bool,
    /// The last answered hits, best first.
    hits: Vec<Hit>,
    /// The highlighted hit.
    cursor: usize,
    /// The last search's own error, drawn in the box (D232).
    error: Option<String>,
    /// A refusal that sent nothing: no projects, no query.
    notice: Option<&'static str>,
    /// The `Ctrl+R` line (D237).
    index: IndexLine,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum IndexLine { #[default] Idle, Running, Done(String), Failed(String) }

impl ConceptsSearch {
    /// Identity of the search; `register_all` registers the factory and `Ctrl+F` under it.
    pub const ID: OverlayId = OverlayId("concepts_search");
    /// An empty search over every project.
    #[must_use]
    pub fn new() -> Self;
}
```

Private helpers: `fn projects(&self, scope: &Scope) -> Vec<ProjectId>` (`None` →
`scope.project_ids.clone()`, `Some(p)` → `vec![p]`); `fn current(&self, scope: &Scope) -> SearchQuery`
= `concepts::query(self.field.text().unwrap_or_default().trim(), self.projects(scope),
self.decisions, concepts::DEFAULT_LIMIT)`; `fn changed(&self, scope: &Scope) -> bool` =
`self.sent.as_ref() != Some(&self.current(scope))`; `fn target(hit: &Hit) -> RevealTarget` (free fn:
`Owner::Item(id)` → `Item { id, key: hit.key.clone() }`, `Owner::Requirement(id)` → `Requirement {
.. }`; `vector.rs:175-181`); `fn clip(text: &str, cells: usize) -> String` (free fn: graphemes via
`crate::ui::cells::graphemes`, summed with `cell_width`, stopping before the one that would pass
`cells`; `cells` is `pub(crate)` in a private module of `ui`, `ui/mod.rs:4`, reachable from
`ui::overlay`).

### 5.3 Keys (`on_key`)

| Key | Effect |
|---|---|
| `Ctrl+D` | `decisions = !decisions` → header changes; `changed()` marks the list stale |
| `Ctrl+P` | `project`: `None` → first of `ctx.projects` → next → … → `None`; a `Some` no longer in `ctx.projects` → `None` |
| `Ctrl+R` | no projects → `notice = Some(NO_PROJECTS)`; else `index = Running` and `ctx.request(IndexConcepts { scope: Scope { workspace_id: ctx.scope.workspace_id, project_ids: self.projects(ctx.scope) } })` |
| any other chord (`modifiers - SHIFT != empty`, e.g. `Ctrl+C`) | `Handled::Pass` (the wildcard `Ctrl+C` quits, `keymap.rs:242-249`) |
| `Up` / `Down` | cursor within `hits`, clamped |
| `field.on_key` → `Consumed` | `notice = None`; `Consumed` (every printable key, `w`, `q`, digits, `?`, `j`, `k`) |
| `Submit` (`Enter`) | `enter(ctx)` |
| `Cancel` (`Esc`) / `Pass` (`Tab`, `F(n)`) | `Handled::Pass`: `Esc` reaches the wildcard close (`keymap.rs:236-241`), the rest is swallowed by `is_modal` (`state.rs:462-465`) |

The chord test is `key.modifiers - KeyModifiers::SHIFT == KeyModifiers::CONTROL` for the three
letters (`'d' | 'D'` etc.), the `ctrl_s` shape (`requirements/mod.rs:285-288`).

`enter(ctx)`, first match wins:
1. `let query = self.current(ctx.scope);` If `Some(&query) != self.sent.as_ref()`:
   - `query.projects.is_empty()` → `notice = Some(NO_PROJECTS)`, nothing sent (D233).
   - `query.text.is_empty()` → `notice = Some(EMPTY_QUERY)`, nothing sent.
   - else `sent = Some(query.clone())`, `searching = true`, `hits.clear()`, `cursor = 0`,
     `error = None`, `notice = None`, `ctx.request(StoreRequest::SearchConcepts(query))`.
2. Else, not `searching`, and `hits.get(cursor)` is `Some(hit)`: `ctx.emit(Action::Overlay(
   OverlayAction::Close)); ctx.emit(Action::Reveal(target(hit)));` (D235: close, then reveal; both
   applied by the same `drain`, `state.rs:333-356`).
3. Else nothing.

### 5.4 Replies (`on_reply`)

`let StoreReply::Concepts(reply) = reply else { return };` then on `&**reply`:
- `Hits { query, outcome }` with `self.sent.as_ref() == Some(query)` (otherwise ignored):
  `searching = false`, `answered = true`; `Ok(hits)` → `self.hits = hits.clone()`, `cursor = 0`,
  `error = None`; `Err(message)` → `hits.clear()`, `error = Some(message.clone())`, `sent = None`.
- `Indexed(Ok(report))` → `index = Done(concepts::report_line(report))`.
- `Indexed(Err(message))` → `index = Failed(message.clone())`.

`wants_requests` → `Vec::new()`; `is_modal` → `true`; `title` → `TITLE`.

### 5.5 Render (the Harness's 100×30; the box is centred over the whole frame)

Constants: `TITLE = "Search concepts"`; `QUERY_LABEL = "query "`; `CURSOR = "> "`, `NO_CURSOR = "  "`
(the switcher's, `workspace_switcher.rs:24-28`); `HIT_ROWS: u16 = 10` (= `DEFAULT_LIMIT`, asserted
equal in a unit test); `BOX_HEIGHT: u16 = 18` (2 borders + 16 rows below); `MARGIN: u16 = 4`;
`MAX_WIDTH: u16 = 120`; `ALL_PROJECTS = "all projects"`; `LOADING = "loading the embedding model…"`;
`SEARCHING = "searching…"`; `INDEXING = "indexing…"`; `IDLE = "type a query; Enter searches"`;
`NO_PROJECTS = "this workspace has no projects"`; `EMPTY_QUERY = "type a query first"`;
`NO_MATCHES = "no matches — Ctrl+R indexes this scope if the index is empty"`;
`CHANGED = "changed — Enter searches again"`; `HINT = "Enter search/open  Up/Dn move  Ctrl+D
decisions  Ctrl+P project  Ctrl+R re-index  Esc close"` (91 cells, fits the 94-cell inner width).

`let width = area.width.saturating_sub(MARGIN).min(MAX_WIDTH); let box_area = centered(area, width,
BOX_HEIGHT);` `Clear`, then `Block::new().borders(Borders::ALL).title(Span::styled(format!("
{TITLE} "), theme.title))`, `inner = block.inner(box_area)`, and
`Layout::vertical([Length(1), Length(1), Length(1), Length(HIT_ROWS), Length(1), Length(1),
Length(1)])`:

```
row 0      header: "scope {all projects|<slug>} · decisions {off|on}"                    dim
row 1      "query " (dim) + field.line(inner.width - 6, true, theme)
row 2      blank
rows 3-12  one per hit: CURSOR|NO_CURSOR + clip(format_hit(hit), inner.width - 2)   accent|base
row 13     search status (below)
row 14     index line: "" | INDEXING (dim) | report line (dim) | error (theme.error), clipped
row 15     HINT (dim), clipped
```

The slug comes from `ctx.projects` (a `ProjectRef` gone from it falls back to the id). The search
status, first match wins: `notice` (dim) → `error` (`theme.error`) → `searching` (`LOADING` if
`!answered` else `SEARCHING`) → `sent == None` → `IDLE` → `hits.is_empty()` → `NO_MATCHES` → `"{n}
hit(s)"`, and on either of the last two `" · " + CHANGED` when `changed(ctx.scope)`. Everything is
clipped to `inner.width` with `clip`.

### 5.6 Data flow, end to end

**Search**:
1. `Ctrl+F` → `App::on_key`: the active tab passes the chord (F7 for Chat), the tab scope has no
   binding, `KeyScope::Global` resolves `Overlay(Open(ConceptsSearch::ID))` (`state.rs:470-506`) →
   `update_overlay` → `push_overlay` (no requests).
2. Letters: `App::on_key` → top overlay's `on_key` with `Ctx { origin: Overlay(concepts_search) }` →
   `TextField` consumes; `Handled::Consumed` stops the chain (`state.rs:432-459`).
3. `Enter` → `enter` → `ctx.request(SearchConcepts(query))` → `Emit` → `App::drain(origin)` →
   `App::dispatch(Origin::Overlay(id), request)`: `seq = next_seq`, `latest[(origin,
   discriminant(SearchConcepts))] = seq`, `RequestEnvelope` on the unbounded channel (`state.rs:302-316`).
4. Worker loop (`store_worker.rs:1597`): the concepts arm → `ConceptsRuntime::serve` (sync) spawns
   the task, aborts a superseded one under the same `(origin, kind)`, answers `Deferred` →
   `continue`; the loop awaited nothing.
5. The task: `QdrantIndex::search` → keyring on the blocking pool → `settings_from` → `embedder()`
   (first time: detached init + `spawn_blocking(FastEmbedder::new)`) → `QdrantStore::connect` →
   `search` → `replies.send(ReplyEnvelope { seq, origin, Concepts(Hits { query, outcome }) })`.
6. `event_loop` → `Action::Reply` → `App::on_reply`: `observe_reply` (nothing), `is_fresh(origin,
   seq)` (dropped if a newer search went out, or if the overlay was closed: D240), not `Failed` so no
   status line, then `overlays.by_id_mut(id)` → `ConceptsSearch::on_reply` (`update.rs:163-234`).

**Reveal**:
7. `Enter` on a hit (not `changed`) → `emit(Overlay(Close))`, `emit(Reveal(target))` → `drain`:
   `Close` → `close_top_overlay` (pops, forgets the overlay's `latest` entries) → `Reveal` →
   `App::reveal`: `reveal_tabs` lookup → `update_tab(Focus(tab))` (a different tab →
   `activate_tab` → its `wants_requests` dispatched under `Tab(tab)`) → `Tab::reveal(&target, ctx)`.
8. Backlog: loaded → `select_item` → unfold + `go` → seven detail reads (`backlog/mod.rs:110-132`).
   Not loaded → `pending_reveal` + a fresh `Items` read that supersedes step 7's → its reply →
   `on_reply(Items)` resolves the pending target before `reselect`, or emits the not-in-backlog
   error. Requirements: the same with `select_row` / `Requirements(scope)` / its notice.
9. `drain(Tab(tab))` dispatches the tab's requests; the next frame shows the tab with the row.

### 5.7 Unit tests (`concepts_search.rs` `mod tests`, a `Bench` like `requirements/mod.rs:1252-1301`)

- `an_empty_scope_sends_nothing_and_says_so`: a `Scope` with no projects; type `x`, `Enter` → no
  `Action::Store` emitted; render text contains `NO_PROJECTS`. Same for `Ctrl+R`.
- `enter_searches_once_then_opens`: first `Enter` emits one `SearchConcepts`; feed the matching
  `Hits { Ok([item hit]) }`; second `Enter` emits `[Overlay(Close), Reveal(Item { .. })]` in that
  order and no `Store`.
- `a_toggle_after_hits_makes_enter_search_again`: after hits, `Ctrl+D` then `Enter` → a new
  `SearchConcepts` with `resolutions == DECISION_RESOLUTIONS`.
- `ctrl_p_cycles_all_then_each_project_then_all`: header reads `all projects`, `htui`, `agy`,
  `all projects`; the sent query's `projects` follows.
- `a_reply_to_another_query_is_ignored`; `a_failed_search_lets_enter_retry` (after `Err`, `Enter`
  sends the same query again).
- `the_first_search_says_it_loads_the_model_later_ones_say_searching`.
- `hit_rows_is_the_cli_default_limit`: `u64::from(HIT_ROWS) == concepts::DEFAULT_LIMIT`.

### 5.8 Integration tests: `crates/htui/tests/concepts_search.rs` (`#![cfg(feature = "testkit")]`)

Helpers:
- `seeded() -> Arc<MemIndex>`: `MemIndex::new()`, then `seed(&Backend::memory(MemStore::demo()),
  &platform_scope)` (the demo is deterministic, so the points match the harness's own
  `MemStore::demo()`).
- `open(index: Option<Arc<MemIndex>>) -> Harness`: `Harness::demo().with_agent_runtime(
  AgentRuntime::new(DriverFactory::new()))`, plus `.with_concepts_runtime(ConceptsRuntime::new(
  index))` when `Some`; `register_all`; `drive_to_end`; `w` `j` `enter`; `drive_to_end`; Platform.
- `type_text` (the `tests/requirements.rs:67-74` helper) and `search(h, text)` = `ctrl-f`, type,
  `enter`, `drive_to_end`.
- The query words are taken from the fixture's own rows (an item title, a document body line of
  `ANA-1`, a requirement body), and expected lines are computed, never hard-coded:
  `clip(format_hit(hit), 92)` prefixes of `index.store().search(&concepts::query(..))`.

| Test | Asserts |
|---|---|
| `ctrl_f_opens_the_search_from_every_tab` | for each of `1`..`5`: key, `drive_to_end`, `ctrl-f` → top overlay id `ConceptsSearch::ID`; `esc` closes. Then `5`, `i` (composer open), `ctrl-f` → open (F7) |
| `typed_letters_and_digits_are_query_text` | type `wq12?jk`: overlay still open, active tab unchanged, `should_quit == false`, the query row shows `wq12?jk` |
| `enter_searches_and_lists_hits_as_the_cli_prints_them` | every expected line's prefix is in the frame, in order; snapshot `concepts_search__hits` |
| `enter_on_an_item_hit_reveals_it_in_the_backlog` | `Down` to the item hit, `enter`, `drive_to_end` → overlays empty, Backlog active, `┌ {KEY} ` |
| `enter_on_a_document_hit_reveals_its_owner_item` | move the cursor to the row containing `document`; `enter` → Backlog, `┌ ANA-1 ` |
| `enter_on_a_requirement_hit_opens_it_in_the_requirements_tab` | → Requirements active, `┌ {R-KEY} ` |
| `ctrl_d_and_ctrl_p_change_the_header_and_mark_the_list_stale` | after hits: `ctrl-d` → header `decisions on`, status contains `CHANGED`; `ctrl-p` → header `scope htui` |
| `a_failing_index_is_an_inline_error_and_the_status_line_stays_empty` | `MemIndex::new().failing("qdrant: query: connection refused")` → the message in the box, `app().status == None`; snapshot `concepts_search__error` |
| `ctrl_r_reindexes_and_prints_the_report_line` | empty `MemIndex`; `ctrl-r`, `drive_to_end` → frame contains `report_line(&expected)`, `expected` from seeding a second empty `MemIndex` over the same scope |
| `without_a_concepts_runtime_the_search_says_not_available` | `open(None)`; search → `NOT_AVAILABLE` in the box, `status == None` |
| `a_reply_to_a_closed_search_is_not_shown_by_a_reopened_one` | `ctrl-f`, type, `enter` (no drive), `esc`, `ctrl-f`, `drive_to_end` → the reopened box shows `IDLE`, no hit rows (D240 end to end) |
| `ctrl_c_still_quits_from_the_search` | `ctrl-f`, `ctrl-c` → `should_quit` |

Snapshots (`insta::assert_snapshot!("<name>", h.render())` → `concepts_search__<name>.snap`):

| Snapshot | Drive |
|---|---|
| `concepts_search__empty` | `open(Some(seeded()))`, `ctrl-f` |
| `concepts_search__searching` | type, `enter`, render **before** any drive: `LOADING` |
| `concepts_search__hits` | after `drive_to_end` |
| `concepts_search__error` | the failing index |
| `concepts_search__decisions_project` | `ctrl-p`, `ctrl-d`, search: header `scope htui · decisions on`, only closed items' hits |

### 5.9 The 27 moved snapshots (F1, D257)

The status row of each becomes `q quit · Tab next tab · Shift+Tab previous tab · 1 select tab · ?
help · w workspaces · Ctrl+f find` (99 cells, not clipped; D257 as amended). They are exactly the files whose last line
ends `w workspaces` at `9a1cf27`:

- `crates/htui/src/snapshots/htui__testkit__tests__shell_empty.snap`;
- under `crates/htui/tests/snapshots/`, 26 files: `integration__demo_shell`;
  `requirements__requirements_{amended,filter,minted,new_area,tree,withdrawn}`;
  `shell__{after_switch,migration_prompt,offline_label,switcher_empty,switcher_open}`;
  `skills__{attach_form_effective_globs,attachments,changed_elsewhere,diff_two_versions,edit,import_report,library,repo_picker}`;
  `templates__{browse,changed_elsewhere,diff_two_versions,edit_help,missing_item_confirm,unknown_placeholder_cursor}`
  (each `.snap`).

Every other snapshot either has no registered global table (`backlog__*`, `chat__*`, the section
suites, which build their shells by hand) or shows an error on the status row, and does not move.
The check is mechanical:

```bash
git diff --stat -- crates/htui/tests/snapshots crates/htui/src/snapshots  # 27 files modified
git status --short -- crates/htui/tests/snapshots | grep '^??'            # the 5 concepts_search__*
git diff -U0 -- crates/htui/tests/snapshots crates/htui/src/snapshots \
  | grep '^[-+][^-+]' | grep -vc 'w workspaces'                          # 0: only status rows moved
```

The reviewer checks each modified file's diff is its last line only.

### 5.10 Gate and commits

```bash
INSTA_UPDATE=always cargo test -p htui --all-features --test concepts_search -- --test-threads=1
# review the 5 new concepts_search__*.snap by eye
INSTA_UPDATE=always cargo test -p htui --all-features -- --test-threads=1   # re-blesses the 27 (§5.9)
cargo test -p htui --all-features -- --test-threads=1                        # green, nothing pending
cargo clippy -p htui --all-features --all-targets -- -D warnings
git diff --exit-code Cargo.lock
```

1. `fix(mod-64): the Chat tab passes chords to the shell` (F7; a unit test in `chat/mod.rs`'s
   `mod tests`: with the composer open, `ctrl-f` answers `Handled::Pass` and the composer text is
   unchanged; the chat suite stays green).
2. `test(mod-64): the concepts search overlay` (red: `ConceptsSearch` with `todo!()` handlers,
   registration without the binding, the unit and integration tests).
3. `feat(mod-64): Ctrl+F opens the concepts search` (green: the overlay, the binding, the 5 new and
   27 re-blessed snapshots, in one commit so no commit has a red snapshot).

---

## 6. Data flow summary

§5.6 is the whole path: key → `Ctx::request` → `App::drain`/`dispatch` (`seq`, `latest[(origin,
kind)]`) → worker arm → `ConceptsRuntime::serve` → spawned task → `ReplyEnvelope` → `App::on_reply`
(`is_fresh`, no `Failed` path) → `ConceptsSearch::on_reply`; and `Enter` → `Overlay(Close)` +
`Action::Reveal` → `App::reveal` → `TabAction::Focus` → `Tab::reveal` → `pending_reveal` resolved on
the next list reply.

---

## 7. Cross-task contracts

| Defined in | Item (exact) | Consumed by |
|---|---|---|
| T1 `htui_store::embed` | `FastEmbedder: Clone` | T2 (`QdrantIndex::embedder`) |
| T1 `htui::concepts` | `query(&str, Vec<ProjectId>, bool, u64) -> SearchQuery`; `report_line(&SyncReport) -> String` | T4, T2 tests |
| T2 `htui::concepts_worker` | `NOT_AVAILABLE`, `REQUEST_NAMES`, `ConceptsReply::{Hits { query, outcome }, Indexed(..)}`, `ConceptIndex`, `QdrantIndex::new`, `MemIndex::{new, failing, delayed, store, seed}`, `ConceptsServed`, `ConceptsRuntime::{new, production, serve, settle, shutdown}` | T4 |
| T2 `htui::store_worker` | `StoreRequest::{SearchConcepts(SearchQuery), IndexConcepts { scope }}`, `StoreReply::Concepts(Box<ConceptsReply>)` | T4 |
| T2 `htui::testkit` | `Harness::with_concepts_runtime(ConceptsRuntime)` | T4 |
| T3 `htui::app` | `Action::Reveal(RevealTarget)`, `RevealTarget::{Item { id, key }, Requirement { id, key }}`, `RevealTarget::{kind, key}`, `RevealKind`, `App::reveal_tabs` | T4 |
| T3 `htui::ui::tabs` | `Tab::reveal(&mut self, &RevealTarget, &mut Ctx<'_>) -> bool`; `registry::CLOSE_THE_FIELD_FIRST`; `backlog::not_in_this_backlog`; `requirements::not_in_these_requirements` | T4 tests |

**Parallel hazards (T1 ∥ T3)**: no shared path; `Cargo.lock` does not move; neither intra-doc-links
the other's items. T2 then T3 merge order is free (T2 ∩ T3 = ∅); both only **add** to `StoreReply`
(T2) and to `Action`/`Tab` (T3), and no file of one matches exhaustively on the other's enum.

---

## 8. Merge order and the workspace gate

T1 ∥ T3 → merge T1 (its gate) → merge T3 (its gate) → T2 → T4 → T5. At close:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features -- --test-threads=1     # with the repo's Postgres test DSN
cargo doc --workspace --no-deps
git diff --exit-code 9a1cf27 -- Cargo.lock
git diff --stat 9a1cf27 -- crates/htui/tests/snapshots crates/htui/src/snapshots   # 27 M, 5 A (§5.9)
```

Then the plan's live check, unchanged (`htui` against the compose Qdrant after `htui
--index-items`; `Ctrl+F`, a query, `Enter` on each hit kind; `Ctrl+R`).

---

## 9. Count pins

| Pin | Before | After | Task |
|---|---|---|---|
| `StoreRequest` / `StoreReply` variants | n / m | n+2 / m+1 | T2 (no test pins either; the only exhaustive matches are `name()` and `try_serve`) |
| `Action` variants | 12 | 13 | T3 (only `App::update` matches it exhaustively) |
| `Tab` trait defaults | 2 | 3 | T3 |
| Registered overlays / global bindings | 2 / `w` | 3 / `w`, `Ctrl+f` | T4 |
| Snapshots | — | +5 new, 27 status rows moved | T4 |
| `Cargo.lock` | — | unchanged | all |
| Migrations, `.sqlx` | — | unchanged | — |

---

## 10. Risks (continuing from the plan's K4)

| # | Risk | Likelihood | Mitigation |
|---|---|---|---|
| K5 | A search in flight when the store worker exits is aborted by `shutdown`; a model load in its detached init task is not (it cannot be) and runs to the end of `spawn_blocking` inside the runtime's shutdown window. | Low | `lib.rs`'s `SHUTDOWN` (5 s) bounds the quit; the load holds no lock or row. *Amended at review (MOD-64 review 1): `SHUTDOWN` bounded only the worker join — `#[tokio::main]`'s runtime drop waited unbounded for the blocking pool, so `main` now builds the runtime and calls `shutdown_timeout(SHUTDOWN)`.* |
| K6 | `Ctrl+F` is taken by a capturing Backlog detail sub-pane that swallows chords (not audited here beyond `TextField`, which passes them). | Low | The from-every-tab test covers each tab in its resting state; a sub-pane that swallows `Ctrl+C` would already break MOD-52. |
| K7 | Hits name an item whose project was folded **and** then removed from the workspace. | Very low | `Items` omits it; the reveal says `not in this workspace's backlog` (the unknown-id path). |

---

## 11. Decisions (D241 onward)

| # | Decision |
|---|---|
| D241 | `StoreRequest::SearchConcepts(SearchQuery)` / `IndexConcepts { scope: Scope }`, names `search_concepts` / `index_concepts`, pinned by `concepts_worker::REQUEST_NAMES`. The overlay builds the query with `concepts::query`, so the worker applies no rule of its own beyond "no projects or no limit answers `[]`". |
| D242 | `StoreReply::Concepts(Box<ConceptsReply>)`; `ConceptsReply::Hits { query: SearchQuery, outcome: Result<Vec<Hit>, String> }`, `ConceptsReply::Indexed(Result<SyncReport, String>)`; `#[derive(Debug, Clone, PartialEq)]` (F2). |
| D243 | `ConceptsServed { Reply, Deferred }` (the `RunServed` precedent); `ConceptsRuntime::serve` is a plain `fn`; tasks keyed by `(Origin, Discriminant<StoreRequest>)` — `App::latest`'s own key — holding `JoinHandle<()>` so `settle` can await them; `settle(limit) -> usize`; `shutdown()` aborts (F3). |
| D244 | The no-runtime answer is `try_serve`'s, `Concepts(..Err(NOT_AVAILABLE))`. `Harness::drive` routes to an installed runtime and otherwise to `store_worker::serve`; `Harness::settle` is unchanged (F3). |
| D245 | `QdrantIndex { embedder: Arc<OnceCell<FastEmbedder>> }`; the init runs on a detached `tokio::spawn` around `get_or_try_init(spawn_blocking(FastEmbedder::new))`; settings are read (blocking pool) before the model; connect errors say `cannot reach Qdrant: …` (F9). |
| D246 | `MemIndex` over an owned `MemVectorStore`, with `failing` and `delayed`, `store()` and `seed()`; `#[cfg(any(test, feature = "testkit"))]`; the `testkit` feature enables `htui-store/test-support`. |
| D247 | D240 lives in `App::forget` (`state.rs`) and `close_top_overlay` / `close_every_overlay` / `forget_overlay` (`update.rs`), on all three pop paths; an id still open elsewhere keeps its entries. Its red tests are shell unit tests in `update.rs` (F8); T4 adds the end-to-end one. |
| D248 | `RevealTarget::{Item { id, key }, Requirement { id, key }}` (F4); `RevealKind::{Item, Requirement}`; `App::reveal_tabs: Vec<(RevealKind, TabId)>`, filled in `register_all`. |
| D249 | The Backlog reports a miss with `Action::Error(not_in_this_backlog(key))`; Requirements with `Notice::Error(not_in_these_requirements(key))` (F5). |
| D250 | `Tab::reveal(&mut self, &RevealTarget, &mut Ctx<'_>) -> bool`, default `false`; `App::reveal` focuses, builds the `Ctx` as `finish_external_edit` does, and drains; an unregistered kind is a logged no-op. |
| D251 | A miss parks `pending_reveal` **and** re-reads the list (so a row created since the last read is found); the next list reply resolves it before `reselect`. A scope change clears it; so does a refused `Requirements` read. |
| D252 | A reveal over a capturing Backlog sub-pane or an open Requirements form does not move the cursor and says `CLOSE_THE_FIELD_FIRST`; the Requirements filter prompt is closed (its text kept as applied) and `unhide` clears it only if it hides the row. |
| D253 | `RequirementsTab::reveal` (private) is renamed `unhide`; `select_row` is `land`'s `:788-797`, byte for byte (F6). |
| D254 | Overlay layout and texts per §5.5; "the first search" is per overlay instance (F10); `ui::cells`-based `clip`, local to the overlay. |
| D255 | `Enter` searches iff `concepts::query(current state) != sent`; sending clears the hits; a failed search clears `sent` so `Enter` retries; otherwise `Enter` opens the highlighted hit. |
| D256 | `ChatTab::on_key` passes `CONTROL`/`ALT` chords before the composer (F7). |
| D257 | **Amended by the maintainer 2026-09-29 (F1):** the help text is `find` (` · Ctrl+f find`, 99 cells, fits the 100-column harness). The 27 status-row snapshots are re-blessed in T4's binding commit, and only their last line may change. |
| D258 | `FastEmbedder: Clone` is asserted at compile time in `htui`'s `concepts.rs` tests (F11). |
| D259 | Every `htui` gate command carries `--all-features` (F12). |
| D260 | `IndexConcepts` with no projects answers `Indexed(Ok(SyncReport::default()))` at once; the overlay never sends one (it says `NO_PROJECTS`). |
| D261 | The overlay's `Esc` is the wildcard binding (the field answers `Cancel` → `Pass`); its only chords are `Ctrl+D`/`Ctrl+P`/`Ctrl+R`, and every other chord passes (so `Ctrl+C` quits). |
| D262 | The worker loop calls `concepts.shutdown()` after the chat and run shutdowns; an index run cut there is repaired by the next run (the index write is idempotent per item). |

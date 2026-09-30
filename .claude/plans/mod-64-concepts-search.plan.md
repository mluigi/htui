# Plan: MOD-64 — concepts search in the TUI

**Status: CONFIRMED by the maintainer 2026-09-29 with OQ-1..OQ-4 defaults as written.** **IMPLEMENTED** `191a7ec`..`6cf2696` (2026-09-30); final review `rust-reviewer` APPROVE WITH FIXES, all eight findings applied. Write-up: `docs/decisions/mod/mod-64.md`.

**Source**: `HANDOFF.md` MOD-64 (from MOD-50, 2026-09-29): a search overlay over the concepts index
— free text, scoped to the selected project or all, a decisions toggle, hits listed as
`concepts::format_hit` prints them; `Enter` on an item or document hit selects that item in the
Backlog, on a requirement hit opens it in the Requirements tab; the search runs off the UI thread;
a missing or unreachable Qdrant is an inline error that affects nothing else. Whether the TUI also
offers a re-index action is this item's call.

**Requirements**: `R-STO-8` (semantic search, decisions narrowing, `docs/REQUIREMENTS.md:162`),
`R-TUI-2` (Backlog left pane, `docs/REQUIREMENTS.md:316`), `R-NF-3` (no store or network work on
the UI thread).

**Routing**: routed as **plan** by `/handoff-run MOD-64` (C3 fired — the re-index question; C4 at
the threshold), 2026-09-29, accepted by the maintainer. Ultracode not needed. Staffing: Opus 5.5
for every agent (architect, implementers, reviewer `rust-reviewer`,
`.claude/workflow-config.json:2`); Fable is not used.

**Numbering**: decisions **D230…** (global; MOD-50 ended at D229). Risks, open questions and tasks
are plan-local: **K1…**, **OQ-1…**, **T1…**.

**Tree**: every fact below was read at `b5e481f` through the Gortex graph (sandbox daemon) and is
listed in *Verified claims*.

---

## Open questions for the maintainer (read these first)

Each has a default this plan adopts, so implementation is not blocked.

- [x] **OQ-1 — Re-index from the TUI, or leave it to MOD-41?** (the item's own question).
      **Default (D237): yes, `Ctrl+R` in the overlay** re-indexes the overlay's current project
      scope with `Indexer::sync` (the same call `htui --index-items` makes), off the UI thread,
      and reports the `SyncReport` counts inline. Reason: MOD-41 (headless worker) is not started,
      so without it a TUI-only user meets an empty index and a pointer to the command line.
      MOD-41 later makes the key mostly unnecessary but not wrong. **Alternative:** no key; the
      empty-result line says `run htui --index-items`.
- [x] **OQ-2 — What "all" and "the selected project" mean.** The shell has no selected project —
      only the workspace scope (`Scope` = one workspace + its projects, `model/scope.rs:10-15`) —
      and a `Hit` carries no project id, so a hit in another workspace could not be selected
      without switching the whole scope. **Default (D233):** "all" = every project of the current
      workspace; `Ctrl+P` cycles *all → project 1 → … → all*, in `ctx.projects` order, shown on
      the overlay's header line. The overlay opens on "all". **Alternative:** "all" spans every
      workspace, and `Enter` on a hit elsewhere switches the scope first (needs a `Hit` →
      workspace lookup the index does not carry).
- [x] **OQ-3 — Key that opens the overlay.** **Default (D236): global `Ctrl+F`.** It is unbound
      everywhere in `crates/htui/src` today; `/` is the Requirements tab's own filter and `s` is
      the Runs pane's "select candidate", and a tab sees a key before the global table, so either
      of those would open search from some tabs and not others. **Alternative:** `/` globally,
      accepting that the Requirements tab keeps its filter on `/`.
- [x] **OQ-4 — Search on `Enter` or as you type.** **Default (D234): `Enter` searches** when the
      query or a toggle changed since the last search; otherwise `Enter` opens the highlighted
      hit. The first search loads the embedding model (seconds; a first-ever run downloads it), so
      typing never triggers work by itself. **Alternative:** live search, debounced on the 250 ms
      tick (needs an `Overlay::on_tick` hook, which the trait lacks).

---

## Summary

A modal **`ConceptsSearch` overlay** (`ui/overlay/concepts_search.rs`, the workspace switcher's
shape) holds a `TextField`, a project-scope cycle, a decisions toggle and the last hit list. It
asks through two new store requests, `SearchConcepts` and `IndexConcepts`, and gets back one new
reply, `StoreReply::Concepts`, which carries **its own error** so a failure stays inside the
overlay and never reaches the status line (D232).

The two requests do not run in the store worker's serial loop (a model load there would stall
every other read). A new **`ConceptsRuntime`** (`concepts_worker.rs`) serves them the way
`AgentRuntime::preview` serves previews: spawn one task per request, abort the one it supersedes,
answer with its own reply (D231). The runtime holds an object-safe **`ConceptIndex`** — the
production one reads the keyring, loads `FastEmbedder` once on the blocking pool and caches it,
and connects to Qdrant per request; the test one wraps `MemVectorStore` (D230). The
harness installs a runtime the way it installs the agent and run runtimes, so the whole path is
testable without Qdrant.

`Enter` on a hit emits a new **`Action::Reveal`**. The shell routes it — item and document hits
to the Backlog tab, requirement hits to the Requirements tab, per registrations in
`register_all` (the `replay_tab` precedent) — focusing the tab and calling a new defaulted
**`Tab::reveal`** (the `focus_section` precedent). Each tab selects the row, unfolding or clearing
a filter that hides it, or keeps the target pending until its next list reply lands (D235).

**Complexity**: medium. No migration, no new crate in `Cargo.lock`, no change to the index
format or `htui-store`'s public API beyond `FastEmbedder: Clone`.

---

## Design decisions (settled here, not in code review)

- **D230 — The search seam is an object-safe trait in `crates/htui`.** `VectorStore` has
  `async fn`s (`vector.rs:344-353`) and is not `dyn`-compatible, and `QdrantStore` is not `Clone`
  (`vector.rs:554-558`). So `concepts_worker.rs` defines
  `trait ConceptIndex: Send + Sync { fn search(&self, query: SearchQuery) -> BoxFuture<'_, Result<Vec<Hit>, String>>; fn sync(&self, backend: Backend, scope: Scope) -> BoxFuture<'_, Result<SyncReport, String>>; }`.
  `QdrantIndex` (production) and `MemIndex` (`#[cfg(any(test, feature = "testkit"))]`, over
  `htui-store`'s `MemVectorStore`, which ranks by shared terms and needs no embedder,
  `vector.rs:776-783`) implement it. `MemIndex` also takes an injectable failure and delay, for
  the error and supersede tests. The `testkit` feature gains `htui-store/test-support` so
  `MemIndex` compiles whenever `testkit` does, not only when dev-dependencies happen to unify it
  in. Errors are display strings: they are only ever shown.
- **D231 — `ConceptsRuntime` serves both requests on spawned tasks, preview-style.** The worker
  loop routes `SearchConcepts` and `IndexConcepts` to `ConceptsRuntime::serve`, which spawns the
  task, keeps its `AbortHandle` per `(Origin, kind)` and aborts the superseded one, and returns
  `Served::Deferred` (`agent_worker.rs:1348-1387`). The task sends its own `ReplyEnvelope`. A
  search never waits for an index run: they are different kinds. `spawn_with_runtimes` builds a
  production `ConceptsRuntime` itself, so its signature — and its test caller at
  `run_worker.rs:2790` — does not change. `Harness::with_concepts_runtime`
  mirrors `with_agent_runtime`; `drive`/`settle` route the two requests to it. Without a runtime
  (harness default) they answer `Concepts { outcome: Err("concepts search is not available") }`.
- **D232 — One reply variant, error inside.** `StoreReply::Concepts(ConceptsReply)` with
  `ConceptsReply::{Hits { hits, query }, Indexed(SyncReport)}` wrapped in `Result<_, String>`.
  `StoreReply::Failed` is deliberately not used: `App::on_reply` turns every fresh `Failed` into a
  status-line `Action::Error` as well (`update.rs:166-179`), and the item says a Qdrant failure
  "affects nothing else". The `Hits` variant echoes the query text and toggles so the overlay can
  tell which search a list answers.
- **D233 — Scope** (OQ-2 default): the project ids sent are the current workspace's, or the one
  cycled to. The overlay never reads another workspace. An empty workspace sends no search and
  says "this workspace has no projects" (the index answers `[]` for no projects,
  `vector.rs:728-773`).
- **D234 — `Enter` searches, then opens** (OQ-4 default). `Up`/`Down` move the hit cursor
  (`TextField` passes them, `text_field.rs:119`); every printable key edits the query, so `j`/`k`
  are text here, not movement. `Ctrl+D` toggles decisions, `Ctrl+P` cycles the project scope,
  `Ctrl+R` re-indexes, `Esc` closes (the wildcard overlay binding). A search in flight shows
  `searching…` (the first one says `loading the embedding model…`); a new search supersedes it.
- **D235 — Reveal is a shell action plus a defaulted tab hook.**
  `Action::Reveal(RevealTarget)`, `RevealTarget::{Item(ItemId), Requirement(RequirementId)}`.
  `App` holds `reveal_tabs: Vec<(RevealKind, TabId)>`, filled in `register_all` (Backlog for
  items, Requirements for requirements), so the shell names no concrete tab. `App::reveal` focuses
  the tab (`TabAction::Focus`, which re-sends its `wants_requests`), then calls
  `Tab::reveal(&mut self, target, ctx) -> bool` — defaulted to `false`, the trait's third default
  after `focus_section` and `on_external_edit`. **Backlog**: unfolds the item's project, moves the
  cursor with `go` if the item is loaded, else parks it in `pending_reveal` and applies it in
  `on_reply(Items)` before `reselect`; an item absent from the reply says
  `<KEY> is not in this workspace's backlog` on the tab's notice. **Requirements**: the same with
  its existing private `reveal` (unfold + clear a hiding filter, `mod.rs:802-819`), `selected` and
  a `RequirementDetail` read — `land`'s tail (`mod.rs:785-797`), factored into one `select_row`.
  A document hit reveals its owner item (the Docs sub-pane is not focused; out of scope).
  The overlay emits `Overlay(Close)` then `Reveal`.
- **D236 — `Ctrl+F` opens the overlay** (OQ-3 default), bound in `register_all` like `w`, help
  text `find` (amended by the maintainer 2026-09-29, blueprint F1: `search concepts` truncates the status row at 100 columns).
- **D237 — Re-index key** (OQ-1 default). `Ctrl+R` sends `IndexConcepts { scope }` for the
  overlay's current projects; the reply prints the `SyncReport` line `htui --index-items` prints,
  factored into `concepts::report_line`. While it runs, searches still work. Closing the overlay
  does not abort it (the index write is idempotent per item); its reply is then dropped with the
  overlay.
- **D238 — The embedder is loaded once, on the blocking pool.** `FastEmbedder` gets
  `#[derive(Clone)]` (its one field is an `Arc`, `embed.rs:25-27`). `QdrantIndex` keeps
  `tokio::sync::OnceCell<FastEmbedder>` initialised through `spawn_blocking(FastEmbedder::new)` —
  the move MOD-34's review deferred (`docs/decisions/mod/mod-34.md:98-99`). A failed load is not
  cached; the next search retries. Settings are re-read from the keyring and Qdrant re-connected
  per request, so a URL changed in Settings > Qdrant applies to the next search.
  *Amended at review (MOD-64 reviews 2, 3): settings are still re-read from the keyring per
  request, but the connected `QdrantStore` is cached keyed by (URL, API key) and reconnected only
  when they change or after a failed call; the model load is one shared future at a time rather
  than a `OnceCell`, so a failed load is retried once, by the next search.*
- **D239 — `concepts.rs` owns the query and the line formats for both front ends.**
  `concepts::query(text, projects, decisions, limit) -> SearchQuery` (the `--decisions` →
  `DECISION_RESOLUTIONS` rule, today inline in `search_items`) and `concepts::report_line`; the
  CLI calls both. The overlay prints `format_hit` verbatim, clipped to the box through `ui::cells`
  (MOD-54). Limit: `DEFAULT_LIMIT` (10).
- **D240 — A popped overlay forgets its freshness entries.** `App::latest` is keyed by
  `(Origin, request kind)` and is not cleared when an overlay is popped, so a reply to a closed
  search overlay would still be "fresh" for a reopened one that has not searched yet. `pop` and
  `clear` on the shell's overlay path drop every `latest` entry whose origin is that overlay. This
  is a shell fix every overlay benefits from; it has its own test.

## Patterns to mirror

| Concern | Mirror |
|---|---|
| Overlay shape, empty/loading text, hint line, box sizing | `ui/overlay/workspace_switcher.rs` |
| Registration + global binding | `app/mod.rs::register_all` (`w` → switcher) |
| Spawned per-request task with supersede-abort | `AgentRuntime::preview`, `agent_worker.rs:1348-1387` |
| Harness installs a runtime | `Harness::with_agent_runtime` / `with_run_runtime`, `testkit.rs` |
| Shell routes a view's intent to a registered tab | `App::replay`, `update.rs:135-142`; `replay_tab` |
| Defaulted `Tab` hook | `Tab::focus_section`, `ui/tabs/registry.rs:58-60` |
| Text entry | `ui::TextField` (`FieldOutcome`), as `settings/qdrant.rs` uses it |
| Overlay tests + snapshots | `crates/htui/tests/shell.rs` (`open_over_demo`, `insta`) |

## Tasks

TDD per repo convention: each task's tests are written first and seen failing.

### T1 — `FastEmbedder: Clone` and the shared line formats (D238, D239)

Files: `crates/htui-store/src/embed.rs`, `crates/htui/src/concepts.rs`.
`#[derive(Clone)]` on `FastEmbedder`; `concepts::query`, `concepts::report_line`, the CLI rewired
to both. Tests: `query` sets `DECISION_RESOLUTIONS` only with `decisions` and keeps the projects;
`report_line` prints today's `index_items` wording byte for byte; existing `format_hit` tests
unchanged.

### T2 — Store request/reply, `ConceptsRuntime`, the index seam, the harness hook (D230–D232, D237, D238)

Files: `crates/htui/src/concepts_worker.rs` (new), `crates/htui/src/lib.rs` (module line only;
the worker is built at `lib.rs:114` through `spawn_with`, whose signature does not change),
`crates/htui/src/store_worker.rs`, `crates/htui/src/testkit.rs`, `crates/htui/Cargo.toml`
(`testkit` feature).
Tests (in-crate, `MemIndex`): a search answers hits ranked by the fake; decisions narrows; no
projects answers `[]` without touching the index; a second search aborts the first (a slow fake
never answers the superseded one); an index run reports its counts; an index error and a search
error both arrive as `Concepts(Err(..))` and never as `Failed`; the default harness answers
"not available"; `QdrantIndex` with no stored URL answers the `settings_from` text (keyring fake
— run under `--test-threads=1`, it is process-wide).

### T3 — Reveal: action, shell routing, tab hooks, freshness fix (D235, D240)

Files: `crates/htui/src/app/action.rs`, `crates/htui/src/app/state.rs`,
`crates/htui/src/app/update.rs`, `crates/htui/src/ui/tabs/registry.rs`,
`crates/htui/src/ui/tabs/backlog/mod.rs`, `crates/htui/src/ui/tabs/requirements/mod.rs`,
`crates/htui/tests/reveal.rs` (new).
Tests (harness over demo): reveal an item already loaded → Backlog focused, cursor on it, detail
reads issued; an item under a folded project → unfolded and selected; before the Backlog has ever
loaded → selected when `Items` lands; an unknown id → the notice, cursor unchanged; a requirement
→ Requirements focused, row selected, filter cleared if it hid it, detail requested; an
unregistered reveal kind → no-op. D240: close an overlay with a request in flight, reopen it, the
old reply is dropped.

### T4 — The overlay, its registration and its tests (D233, D234, D236)

Files: `crates/htui/src/ui/overlay/concepts_search.rs` (new),
`crates/htui/src/ui/overlay/mod.rs`, `crates/htui/src/app/mod.rs`,
`crates/htui/tests/concepts_search.rs` (new), `crates/htui/tests/snapshots/*concepts_search*`
(new).
Tests (harness + `MemIndex` seeded with demo item/requirement/document points): `Ctrl+F` opens it
from every tab; typed `w`/`q`/digits land in the field (modal); `Enter` searches and lists
`format_hit` lines; `Enter` again on a hit reveals it and closes the overlay (item, document →
owner item, requirement); `Ctrl+D` / `Ctrl+P` change the header and mark the list stale; an
error renders inline in the box and the status line stays empty; `Ctrl+R` shows the report line.
Snapshots: empty, searching, hits, error, decisions-on with a project scope.

### T5 — Close-out (main thread)

`docs/decisions/mod/mod-64.md`, `DECISIONS.md` line, HANDOFF line removed and the summary table
recounted, `MOD-41`'s HANDOFF note amended (the TUI now has a manual re-index, if OQ-1 stands),
this plan's status; validator green.

### Independence (file-set intersection)

T3 also touches `crates/htui/src/app/mod.rs` (the `reveal_tabs` registrations).

| | T1 | T2 | T3 | T4 |
|---|---|---|---|---|
| T1 | — | ∅ | ∅ | ∅ |
| T2 | ∅ | — | ∅ | ∅ |
| T3 | ∅ | ∅ | — | `app/mod.rs` |
| T4 | ∅ | ∅ | `app/mod.rs` | — |

**T1 ∥ T3** (disjoint, no dependency). **T2 needs T1** (`FastEmbedder: Clone`, `concepts::query`)
and is disjoint from T3, so it can start as soon as T1 lands, alongside T3. **T4 needs T2 and T3**
(the reply type, `Action::Reveal`, the harness hook) and shares `app/mod.rs` with T3, so it runs
last. Order: T1 ∥ T3 → T2 → T4 → T5.

## Test plan

- `cargo test -p htui --features testkit` (new: `tests/reveal.rs`, `tests/concepts_search.rs`,
  in-crate `concepts_worker` and `concepts` tests) and `cargo test -p htui-store` for `embed.rs`.
- Gate as the repo runs it: `cargo test --workspace -- --test-threads=1` (the keyring fake is
  process-wide), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
- Live check (maintainer, host): `htui` against the compose Qdrant after `htui --index-items`;
  `Ctrl+F`, a query, `Enter` on each hit kind. The real-model path is not unit-tested (the
  only real-embedder test is `#[ignore]`, `embed.rs:170`).

## Risks

- **K1 — First search is slow and silent.** `FastEmbedder::new` downloads BGE-small into
  `<cache>/htui/fastembed` on first use, then loads it (`embed.rs:40-56`); size and load time were
  not measured here (the only real-model test is `#[ignore]`). Mitigated by the
  `loading the embedding model…` line; not a progress bar.
- **K2 — `Send` of the spawned futures — closed by the fact-check.** `DenseEmbedder::embed` and
  `VectorStore`'s methods are `async fn`s in traits with no `Send` bound (`embed.rs:14-20`,
  `vector.rs:344-353`), but `tokio::spawn` of a search over `QdrantStore<FastEmbedder>` and of
  `Indexer::sync` over `Backend` + `QdrantStore<FastEmbedder>` or `MemVectorStore` compiles
  (probe below): the concrete types leak `Send`.
- **K3 — Reveal lands on a tab that has not loaded.** Handled by `pending_reveal`; the test
  "before the Backlog has ever loaded" pins it.
- **K4 — Stale index.** Hits can name an item deleted since the last sync; the reveal says so
  (T3's unknown-id test) instead of failing silently.

## Validation

`bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` at close-out; the gate above
green with `--test-threads=1`.

## Acceptance

1. `Ctrl+F` opens the search from every tab; typed text never triggers a global key.
2. Hits read exactly as `htui --search-items` prints them, for the same query and scope.
3. `Enter` on an item or document hit leaves the Backlog focused with that item selected; on a
   requirement hit, the Requirements tab with that requirement open.
4. With no Qdrant URL stored, or Qdrant down, the error is inside the overlay; the status line,
   the tabs and the store worker's other replies are untouched.
5. No store or network work runs on the UI thread; the worker's serial loop never awaits a search.
6. (OQ-1 default) `Ctrl+R` re-indexes and reports the counts.

## Verified claims

Plan fact-check (step 3.5), 2026-09-29, at `b5e481f`. Tree facts through the Gortex graph and
`grep`; toolchain facts by a compile probe (`crates/htui/examples/mod64_probe.rs`, `cargo check -p
htui --example mod64_probe`, deleted after).

| Claim | Verdict | Evidence |
|---|---|---|
| The store worker serves requests serially; a slow inline arm stalls every other request | ✓ | `store_worker.rs:1526-2027`, one `select!` at `:1595`, arms `.await`ed inline |
| A spawned per-request task with supersede-abort already exists to mirror | ✓ | `AgentRuntime::preview`, `agent_worker.rs:1348-1387` (`previews.insert(origin, abort_handle)`, `Served::Deferred`) |
| A fresh `StoreReply::Failed` also goes to the status line | ✓ | `App::on_reply`, `update.rs:166-179` — hence D232's own reply variant |
| Overlay replies route by `Origin::Overlay(id)`; popped overlays drop them | ✓ | `update.rs:208-234`, `by_id_mut` |
| `latest` is not cleared when an overlay is popped (D240) | ✓ | `update_overlay`, `update.rs:77-89`; `dispatch`/`is_fresh`, `state.rs:302-327` |
| `VectorStore` is not `dyn`-compatible; `QdrantStore` is not `Clone` | ✓ | `vector.rs:344-353` (`async fn`), `:554-558` (no derive) |
| `FastEmbedder`'s only field is an `Arc`; it is not `Clone` today | ✓ | `embed.rs:25-27`; probe: `E0599 no method named clone` |
| `FastEmbedder::new` is synchronous (download + load on the calling thread) | ✓ | `embed.rs:40-56`; `docs/decisions/mod/mod-34.md:98-99` |
| Spawned search / sync futures are `Send + 'static` (K2) | ✓ | probe: `tokio::spawn` of `QdrantStore<FastEmbedder>::search`, `Indexer::sync(&Backend, …, &QdrantStore<FastEmbedder>)` and `…&MemVectorStore` compiles |
| `MemVectorStore` is generic over an embedder | ✗ → amended | `vector.rs:776-783`: not generic, ranks by shared terms; plan now says `MemVectorStore` |
| `MemVectorStore` reachable from `htui` under `testkit` | ~ → amended | only via the dev-dependency (`Cargo.toml:70`); `testkit = []` (`:23`); D230 adds `htui-store/test-support` |
| The search returns `[]` for no projects | ✓ | `QdrantStore::search`, `vector.rs:728-773` |
| `Hit` carries no project id | ✓ | `vector.rs:322-339` — basis of OQ-2 |
| The shell has no selected project, only `Scope` + `projects` | ✓ | `model/scope.rs:10-15`; `App`, `state.rs:133-208`; `Ctx`, `state.rs:73-88` |
| No view can be told to select an entity by id; no cross-view navigation exists | ✓ | `Tab` trait `registry.rs:35-64`; Backlog `go` private (`backlog/mod.rs:110-132`); `RequirementsTab::ID` referenced only by a test |
| `Tab` has two defaulted hooks to mirror | ✓ | `focus_section`, `on_external_edit`, `registry.rs:58-63` |
| Requirements has a private reveal (unfold + clear filter) and `land`'s select tail | ✓ | `requirements/mod.rs:802-819`, `:785-797` |
| Backlog folds by `ProjectId` | ✓ | `list::rows`, `backlog/list.rs:77-90` |
| Shell routes a view's intent to a registered tab | ✓ | `App::replay`, `update.rs:135-142`; `replay_tab` set in `register_all` |
| `Ctrl+F` is unbound in `crates/htui/src` | ✓ | no `Char('f')` binding anywhere (grep); global table `keymap.rs:205-254` |
| `/` and `s` are taken by views | ✓ | Requirements filter `requirements/mod.rs:497`; Runs pane `s`, `backlog/detail/runs.rs:2170` |
| `Ctrl+D`/`Ctrl+P`/`Ctrl+R` free inside an overlay | ✓ | overlay-scope bindings are `Esc` and `Ctrl+C` only (`keymap.rs:205-254`); no `CONTROL` + `d`/`p`/`r` in `crates/htui/src` |
| `TextField` passes `Up`/`Down` and non-SHIFT chords | ✓ | `text_field.rs:119` |
| The harness never runs the worker loop; runtimes are installable | ✓ | `testkit.rs:250-387` (`drive`), `with_agent_runtime`; `try_serve` has no wildcard (`store_worker.rs:1251-1408`) |
| The worker is built in `lib.rs`, and `spawn_with_runtimes` has a test caller | ✓ → amended | `lib.rs:114` (`spawn_with`), `run_worker.rs:2790`; T2 no longer names `main.rs` |
| `Indexer::sync` takes `&impl ReadStore` and works over `Backend` | ✓ | `vector_sync.rs:67-71`; `impl ReadStore for Backend`, `backend.rs:611`; `Backend: Clone`, `:50-70` |
| No background sync exists (MOD-41 not started) | ✓ | only caller `concepts::index_items`, `concepts.rs:128-145`; `HANDOFF.md` MOD-41 note |
| Task independence | ✓ | file sets in *Tasks*; only T3 ∩ T4 = `app/mod.rs`, and T4 runs after T3 |
| First model download size / load time | unverified | not measured; K1 worded without a number |

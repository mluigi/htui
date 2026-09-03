# Plan: MOD-1 TUI scaffold

**Source PRD**: `.claude/prds/mod-1-tui-scaffold.prd.md`
**Selected Milestone**: all four (1 Shell boots, 2 Store seam, 3 Backlog tab, 4 Scope and stubs).
One plan because the milestones are one skeleton and the maintainer asked for a skeleton in one
pass; each milestone maps to a wave below so the PRD rows can still be ticked individually.
**Complexity**: Medium
**Routing**: PRD path (C2 + C4), ultracode accepted for implement and review. Reviewer:
`rust-reviewer` (`.claude/workflow-config.json`). All agents run on Opus per maintainer.

## Summary
Create a Cargo workspace with two crates: `htui-core` (domain types mirroring ANA-9 §5, the §6.1
`ReadStore`/`WriteStore` traits, `Backend`, `MemStore`, demo fixtures, a trait-level conformance
suite) and `htui` (the binary: terminal lifecycle, tokio event loop, store worker, registration
based tabs / sub-tabs / overlays / keymap, top bar, Backlog tab with five read-only sub-tabs,
Skills and Settings stubs, workspace switcher). Views never touch the store; they render snapshots
delivered by message from a store worker task (`R-NF-3`).

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D1 | Cargo workspace, crates `crates/htui-core` (lib) and `crates/htui` (bin) | MOD-6 adds `htui-store` (Postgres + SQLite behind features, ANA-9 §10) without touching the TUI crate; MOD-2 adds the driver crate the same way. |
| D2 | Native `async fn` in traits, exactly the §6.1 syntax; `Backend` is a concrete enum that implements `ReadStore` by delegation. The two traits carry `#[allow(async_fn_in_trait)]` with a comment pointing here | Matches ANA-9 verbatim. Concrete `Backend` keeps futures `Send`-inferable for `tokio::spawn`; no `async_trait` boxing. rustc 1.98 warns on `async fn` in public traits (V3), and the crate builds with `-D warnings`, so the allow is deliberate, not an oversight. MOD-1 variant: `Backend::Memory(MemStore)`; MOD-6 adds `Online` / `Offline`. |
| D3 | Domain types carry §5 column names; IDs are newtypes over `Uuid` (v7 minted client-side, §3) | Zero mapping surprises in MOD-6; type-level separation of `ItemId` / `ProjectId` etc. |
| D4 | Store worker task: UI sends `StoreRequest` on an mpsc, worker awaits the store and replies with `StoreReply`; the UI loop `select!`s over terminal events, replies and a tick | `R-NF-3` by construction: no store handle exists on the render side. |
| D5 | Elm-style `App` state + `Action` enum + `update` + `view`; tabs, detail sub-tabs and overlays are trait objects in registries; key bindings are data (`Keymap`) resolved by scope | PRD extensibility metric: MOD-2/13/14/15 add a `Box<dyn Tab>` / `Box<dyn DetailTab>` / overlay / binding, never a match arm in the loop. |
| D6 | `MemStore` enforces §4.1 (counter per `(project, prefix)`, no delete, `key_prefix` copied at mint) and §4.2 (CAS on `version`, `Diverged { head, ancestor }`, status CAS never bumps `version`) | The conformance suite is the one MOD-6 will run against `PgStore`; behaviour is fixed now. |
| D7 | Demo fixtures behind `--demo`; default `MemStore` is empty | Maintainer decision. |
| D8 | Panic hook + drop guard restore the terminal; `ratatui::init()` / `ratatui::restore()` | PRD risk "terminal left raw". |
| D9 | Tests: `cargo test` in both crates; TUI snapshots via `ratatui::backend::TestBackend` + `insta`; core via conformance suite over `MemStore` | No existing convention; these are the ratatui community defaults. |
| D10 | Scope is always a workspace; `Scope { workspace_id, project_ids }` built from `workspace_project` rows | Maintainer decision; `R-ENT-2` fallback not surfaced. |
| D11 | Runs, Graph, Documents, Notes sub-tabs render what `ReadStore::runs / links(hops = 1) / documents / notes` return, read-only, no key actions beyond scroll | Skeleton only; MOD-4 / MOD-14 / MOD-13 add actions through the registries. |

## Patterns to Mirror
| Category | Source | Pattern |
|---|---|---|
| Naming | — | **No code exists in the repo.** Conventions set here: snake_case modules, one type per file where the type is public, `crates/<name>/src/<area>/mod.rs` for registries. |
| Errors | — | None to mirror. `thiserror` enum `StoreError` in core; `anyhow::Result` in the binary's main only. |
| Logging | — | None to mirror. `tracing` with `tracing-subscriber` to a file under the scratch dir when `HTUI_LOG` is set; never to stdout (it is the TUI). |
| Data access | `docs/ANA-9.md:824-847` | `ReadStore` / `WriteStore` / `Backend` shape, copied not paraphrased. |
| Tests | — | None to mirror; D9. |

## Files to Change
All CREATE. Grouped by task; the task file sets are the independence facts for §3.5.

| File | Task | Why |
|---|---|---|
| `Cargo.toml` (workspace), `rust-toolchain.toml`, `.gitignore`, `rustfmt.toml`, `clippy.toml` | T1 | Workspace root, MSRV pin (1.85 edition 2024), `target/` ignore |
| `crates/htui-core/Cargo.toml`, `src/lib.rs` | T1 | Library crate |
| `crates/htui-core/src/model/{mod,ids,user,box_,hierarchy,kind,item,link,note,document,run,event,scope}.rs` | T1 | §5 tables as types, §4.3 event kinds, `Scope`, `ItemFilter` |
| `crates/htui-core/src/store/{mod,error,traits,backend}.rs` | T1 | §6.1 traits, `UpdateOutcome`, `Backend` enum |
| `crates/htui-core/src/store/mem.rs` | T2 | `MemStore: WriteStore` |
| `crates/htui-core/src/store/conformance.rs` (feature `test-support`) | T2 | Trait-level suite MOD-6 reuses |
| `crates/htui-core/src/fixtures.rs` (feature `demo`) | T2 | Demo data per `R-ENT-6` seed + a run with events |
| `crates/htui-core/tests/mem_store.rs` | T2 | Runs the conformance suite over `MemStore` |
| `crates/htui/Cargo.toml`, `src/main.rs`, `src/cli.rs` | T3 | Binary, `--demo`, log setup |
| `crates/htui/src/terminal.rs` | T3 | init / restore / panic hook guard |
| `crates/htui/src/app/{mod,state,action,update}.rs` | T3 | `App`, `Action`, `update` |
| `crates/htui/src/store_worker.rs` | T3 | `StoreRequest` / `StoreReply`, worker task |
| `crates/htui/src/event_loop.rs` | T3 | `select!` over crossterm `EventStream`, replies, tick |
| `crates/htui/src/keymap.rs` | T3 | `Keymap`, `Scope`, binding table, help line |
| `crates/htui/src/ui/{mod,theme,top_bar,layout}.rs` | T3 | Frame layout, top bar |
| `crates/htui/src/ui/tabs/{mod,registry}.rs` | T3 | `Tab` trait, `TabRegistry`, tab strip |
| `crates/htui/src/ui/tabs/{skills,settings}.rs` | T3 | Stub tabs |
| `crates/htui/src/ui/overlay/{mod,registry}.rs` | T3 | `Overlay` trait, overlay stack |
| `crates/htui/src/ui/tabs/backlog/{mod,list,detail/mod,detail/body,detail/runs,detail/graph,detail/documents,detail/notes}.rs` | T4 | Backlog tab, `DetailTab` trait + registry, five sub-tabs |
| `crates/htui/tests/snapshots/backlog.rs` + `snapshots/*.snap` | T4 | Snapshot tests |
| `crates/htui/src/ui/overlay/workspace_switcher.rs` | T5 | Switcher overlay |
| `crates/htui/tests/snapshots/shell.rs` + `snapshots/*.snap` | T5 | Top bar + switcher snapshots |
| `crates/htui/src/ui/tabs/mod.rs`, `crates/htui/src/ui/overlay/mod.rs`, `crates/htui/src/app/mod.rs` | T6 | Register backlog tab and switcher; wire top bar fields |
| `README.md` | T6 | Build / run / `--demo` / platform notes (Windows Terminal target, conhost best-effort) |

## Tasks

Waves: A = {T1, T2} serial (T2 needs T1's types); B = {T3} serial (needs core API);
C = {T4, T5} **parallel** (disjoint file sets, verified below); D = {T6} serial (touches the
registration files both C tasks would otherwise share). Ultracode runs C as the fan-out; A, B, D
are single-agent stages in the same script. TDD per task: tests first, then code.

### Task 1: workspace + core model + store traits
- **Action**: workspace `Cargo.toml` (resolver 3, shared `[workspace.dependencies]`: `uuid` 1 with
  `v7`+`serde`, `chrono` 0.4 with `serde`, `serde` 1, `serde_json` 1, `thiserror` 2, `tokio` 1
  with `rt-multi-thread,sync,macros,time`, `ratatui` 0.30, `crossterm` 0.29 with `event-stream`,
  `clap` 4 with `derive`, `tracing` 0.1, `tracing-subscriber` 0.3, `anyhow` 1, `insta` 1).
  `htui-core` model types: every §5 table that the TUI reads, with §5 column names; enums as Rust
  enums with `as_str()` matching the `CHECK` lists (`Status`, `LinkKind`, `RunKind`, `RunMode`,
  `RunStatus`, `StepStatus`, `GateOutcome`, `EventKind`, `EventRole`, `OsFamily`, `Transport`,
  `Billing`). `Scope`, `ItemFilter` (all fields optional: statuses, project_ids, tags, ready),
  `ItemSummary`, `Item`, `LinkGraph { nodes, edges }`, `DocumentHead`, `Note`, `RunSummary`,
  `SessionEvent`, `NewItem`, `ItemPatch`, `UpdateOutcome`, `ItemRevision`. Store traits copied
  from §6.1; `StoreError`; `Backend` enum with `Memory` variant and delegating `ReadStore` impl.
- **Mirror**: `docs/ANA-9.md` §3 conventions, §4.3 kinds, §5 columns, §6.1 traits.
- **Validate**: `cargo build -p htui-core` and `cargo clippy -p htui-core -- -D warnings`.

### Task 2: MemStore, conformance suite, fixtures
- **Action**: `MemStore` over `std::sync::RwLock<State>` (never held across an await): maps keyed
  by ID, `item_key_counter: HashMap<(ProjectId, String), i32>`, revisions, links with tombstones,
  notes, documents, runs, steps, events. `mint_item` per §7.1 semantics (counter upsert, version 1,
  revision `created`); `update_item` per §7.2 (`Diverged { head, ancestor }` on version mismatch,
  revision on success, `version` covers exactly the §4.2 spec columns); `transition` CAS on
  status only. `links(id, hops)` BFS over live edges across projects. Conformance suite:
  `pub async fn run_all<S, F>(make: F)` with named cases (consecutive keys per prefix, prefix
  isolation, CAS diverged with ancestor version, status CAS does not bump version, no-delete,
  hops 1 vs 2, filter by status / project / tags / ready, documents ordered by version, notes
  ordered by created_at, events ordered by seq). Fixtures (`feature = "demo"`): one user, one box,
  two workspaces (one with two projects, one with a single project), kinds + graphs + templates
  per `R-ENT-6` / §5.10, about a dozen items across statuses with `blocked_by` / `origin` /
  `relates` links, notes, documents, one finished run with steps and a short §4.3 event stream.
- **Mirror**: `docs/ANA-9.md` §4.1, §4.2, §5.10, §7.1, §7.2.
- **Validate**: `cargo test -p htui-core --features test-support,demo`.

### Task 3: TUI shell
- **Action**: `main.rs` parses `--demo`, builds `Backend::Memory(MemStore::demo() | ::new())`,
  starts tokio, spawns the store worker, runs the event loop, restores terminal. `terminal.rs`:
  `ratatui::init()` / `restore()` guard plus panic hook. `store_worker.rs`: `StoreRequest`
  (`Workspaces`, `Items { scope, filter }`, `Item(id)`, `Links { id, hops }`, `Documents(id)`,
  `Notes(id)`, `Runs(id)`) and `StoreReply` with a request `seq` so stale replies are dropped;
  worker owns `Backend`. `App` holds `Scope`, `TopBarState` (workspace name, box hostname,
  `PgState::Memory`, active runs), `TabRegistry`, `OverlayStack`, `Keymap`, `ReplySink`.
  `Action` enum: `Quit`, `Tab(TabAction)`, `Overlay(OverlayAction)`, `Store(StoreRequest)`,
  `Reply(StoreReply)`, `Tick`. `Tab` trait: `id()`, `title()`, `on_key(&mut self, KeyEvent,
  &mut Ctx) -> Handled`, `on_reply(&mut self, &StoreReply, &mut Ctx)`, `render(&self, Frame, Rect,
  &Ctx)`, `wants_requests(&self, &Scope) -> Vec<StoreRequest>` (called on activation and scope
  change). `Overlay` trait mirrors it plus `is_modal()`. `Keymap`: `Vec<Binding { scope, key,
  action, help }>`; global bindings `q`, `Tab`/`Shift+Tab`, `1..9`, `w` (workspace switcher,
  registered by T5), `?` help line. Stub tabs Skills and Settings render a centred "not yet"
  paragraph. Event loop: `tokio::select!` over `EventStream`, reply channel, 250 ms tick;
  render after every action; resize handled by ratatui autoresize.
- **Mirror**: D4, D5, D8.
- **Validate**: `cargo build -p htui`, `cargo clippy -p htui -- -D warnings`, `cargo test -p htui`
  (shell snapshot: empty store renders top bar + tab strip + stub tab).

### Task 4: Backlog tab (parallel with T5)
- **Action**: `BacklogTab: Tab` with left list and right detail. List: items of the scope grouped
  by project (project header rows, item rows `KEY  status  title`), `j/k`/arrows, `g/G`, group
  fold on `Enter` over a header, project order by `workspace_project.position`, items by
  `key_number`. Selection change enqueues `Item`, `Runs`, `Links{hops:1}`, `Documents`, `Notes`
  requests. Detail: `DetailTab` trait (`id`, `title`, `on_key`, `on_reply`, `render`) and a
  `DetailRegistry` holding Body, Runs, Graph, Documents, Notes in that order; `h/l` or `[`/`]`
  cycle sub-tabs. Body: title, key, kind, status, tags, priority, `version`, markdown body as
  wrapped paragraph with scroll. Runs: table of `RunSummary` (kind, mode, status, box, started,
  finished). Graph: flat table of `LinkGraph.edges` at one hop (direction arrow, kind, other
  item's key + status). Documents: list of `DocumentHead` (kind, version, title, produced by
  step or hand). Notes: chronological thread. Every sub-tab handles the empty case with one
  line, not a blank pane.
- **Mirror**: `Tab` / registry contract from T3; D11.
- **Validate**: `cargo test -p htui` snapshot tests: demo scope grouped list, each sub-tab with
  data, each sub-tab empty.

### Task 5: Workspace switcher overlay + top bar wiring (parallel with T4)
- **Action**: `WorkspaceSwitcher: Overlay`, opened by `w`, lists workspaces from the
  `Workspaces` reply with project counts, `Enter` selects and emits `Action::SetScope(scope)`
  (scope built from `workspace_project`), `Esc` closes. `App::update` on `SetScope` updates
  `TopBarState.workspace`, clears tab caches through `Tab::on_scope_change`, and re-issues
  `wants_requests`. Top bar renders `workspace · box · store state · N runs`; store state text
  comes from `Backend::label()` (`"memory"` now; MOD-6 adds online / offline / cache age).
  Startup with `--demo` selects the first workspace automatically; empty store shows the
  switcher open with "no workspaces" (creation is MOD-15).
- **Mirror**: `Overlay` contract from T3; D10.
- **Validate**: `cargo test -p htui` snapshot tests: switcher open with two workspaces, top bar
  after switch, empty store state.

### Task 6: registration, README, cross-platform check
- **Action**: register `BacklogTab` first in `TabRegistry`, `WorkspaceSwitcher` in the overlay
  registry, `w` binding in the keymap; README with build / run / `--demo` / key table / platform
  notes; run `cargo build --release` on Windows (this box) and at least `cargo check` with
  `--target x86_64-unknown-linux-gnu` if the target is installed, else record as manual follow-up.
- **Mirror**: T3 registries.
- **Validate**: full `cargo test --workspace --all-features`, `cargo clippy --workspace
  --all-targets -- -D warnings`, `cargo fmt --check`, manual `cargo run -- --demo` walkthrough on
  Windows Terminal.

## Validation
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo run -p htui -- --demo
```

## Verified claims (§3.5 fact-check)
Filled by the fact-check step before CONFIRM. `tree` = grep/read of the repo; `probe` = compile
probe against rustc 1.98.1 in the session scratch dir.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| V1 | No Rust code exists in the repo (pattern grounding is empty by fact, not omission) | **true** | `find . -name '*.rs' -o -name Cargo.toml` → 0 files (2026-09-03) |
| V2 | `docs/ANA-9.md` §6.1 trait text matches the signatures copied in T1 | **true** | `docs/ANA-9.md:824-847` read; T1 lists the same seven `ReadStore` and three `WriteStore` methods plus `UpdateOutcome` |
| V3 | Native `async fn` in a trait with `Send + Sync` supertraits compiles on rustc 1.98.1, and a concrete enum delegating to it can be awaited inside `tokio::spawn` (Send inferred) | **true, with a lint** | scratch probe built and printed `probe ok`; rustc emitted `warning: use of async fn in public traits is discouraged as auto trait bounds cannot be specified` → D2 amended with `#[allow(async_fn_in_trait)]` |
| V4 | ratatui 0.30 exposes `ratatui::init()` / `ratatui::restore()` and `backend::TestBackend`; crossterm 0.29 `EventStream` exists behind `event-stream` | **true** | same probe: `ratatui::init` / `ratatui::restore` bound as fn items, `TestBackend::new(100, 30)` drew a frame, `crossterm::event::EventStream::new().next()` compiled (needs `mut`, `futures::StreamExt`) with ratatui 0.30.2 / crossterm 0.29 |
| V5 | `uuid` 1.26 with feature `v7` provides `Uuid::now_v7()` | **true** | same probe |
| V6 | Task file sets: T4 ∩ T5 = ∅ (the only shared files are deferred to T6) | **true** | T4 = `ui/tabs/backlog/**`, `tests/snapshots/backlog.rs`; T5 = `ui/overlay/workspace_switcher.rs`, `tests/snapshots/shell.rs`; `ui/tabs/mod.rs`, `ui/overlay/mod.rs`, `app/mod.rs` are T6 only. Snapshot `.snap` files are per test file, no overlap |
| V7 | MOD-13 / MOD-14 / MOD-15 / MOD-4 / MOD-12 in `HANDOFF.md` cover every `R-TUI-2` action and `R-TUI-5`, `R-TUI-8`, `R-TUI-9` | **true** | `HANDOFF.md` 2026-09-03 edits: `new`/`edit` + filters → MOD-13; `open graph` → MOD-14; `run`/`close` + `R-TUI-9` → MOD-4; `queue` → MOD-12; `R-TUI-8` sections → MOD-2/7/10/12/15; validator green |

## Risks
| Risk | Likelihood | Mitigation |
|---|---|---|
| ratatui 0.30 API moved (workspace split into `ratatui-core` / `ratatui-widgets`) and probe shows a different entry point | Medium | V4 probe before CONFIRM; adjust T3 to the real API |
| Native async-fn-in-trait futures not `Send` in a generic context | Medium | D2 keeps the spawned task over the concrete `Backend`; conformance suite runs inline without `spawn`; V3 probe |
| Parallel agents both edit registration files | Medium | Registration moved to T6; V6 |
| Snapshot tests flaky across terminal sizes | Low | `TestBackend` with fixed 100x30 |
| `std::sync::RwLock` held across `.await` in `MemStore` | Low | Reviewer checklist item; the async fns lock, clone, drop, then return |

## Acceptance
- [ ] All tasks complete
- [ ] Validation passes on Windows; Linux/macOS build recorded
- [ ] Patterns mirrored, not reinvented (ANA-9 §5 / §6.1 verbatim)
- [ ] `rust-reviewer` findings applied or deferred with the maintainer
- [ ] PRD milestone rows updated

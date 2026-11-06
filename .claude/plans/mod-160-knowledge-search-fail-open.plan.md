# Plan: MOD-160 — Knowledge search fails open cheaply and says so

**Source**: HANDOFF `MOD-160` (from ANA-37, `docs/ANA-37.md` §7.1, §7.4, §8 Q6; §2.4 lists the three
defects). Decision record: `docs/decisions/ana/ana-37.md`. Builds on MOD-158
(`docs/decisions/mod/mod-158.md`: `resolve_settings`, the worker's start-time `search_concepts` gate).
**Routing**: plan (C4 fired, 1 of 4), ultracode for the implement phase; accepted by the maintainer
2026-11-05. Sandbox run `hr/MOD-160`.
**Requirements**: `R-NF-2`, `R-NF-3`, `R-STO-8`, `R-MCP-2`, `R-TUI-8`. Their text already says this
("knowledge search unavailable", the tool not offered while no Qdrant is configured, the Settings
warning); no requirement edit.
**Complexity**: Large (store, TUI index, MCP host, worker, overlay, Settings, CLI, gate).
**Status**: confirmed 2026-11-05. Design questions Q1-Q4 answered by the maintainer 2026-11-05, all
as recommended (D5, D7, D10, D9).
**Done 2026-11-06** on `hr/MOD-160`: T1-T6 plus review fixes R1; write-up `docs/decisions/mod/mod-160.md`
(D7 hint-row deviation accepted; R1 adds `QdrantCheck::Unreadable`).

## Summary

Today a TUI search against a Qdrant that is down can download the 133 MB model first, then fail; it
uses `QdrantStore::connect`, so a search also **creates** the collection on an empty Qdrant
(`concepts_worker.rs:535-551, 573-582`). The TUI offers `search_concepts` to every session whether or
not a Qdrant URL is set (`mcp_search.rs:87-93`, `lib.rs:249-254`), and the worker decides once at
start (`worker_cmd.rs:117-133`). Failures read differently on each surface ("search unavailable:
…" for agents, a raw error in the overlay, error codes on the CLI), and nothing warns that the
workspace has no Qdrant. The live Qdrant suites pass silently when no test Qdrant is set.

This item makes every search ask Qdrant (and the collection) before the model and never create a
collection, as the CLI already does (`concepts.rs:696-727`); gives every search surface one sentence,
"knowledge search unavailable", with its cause one key away (TUI) or after a colon (CLI, MCP);
offers `search_concepts` per session from a cached "URL configured" flag; warns, never blocks, in
Settings, at TUI launch and at worker start; and adds a `qdrant` gate step.

Out of scope: MOD-159 (collections per workspace, the indexer lease, the 60 s cycle, workspace scope
for agents); MOD-161 (related items); a fallback text search for agents (MOD-112 (1), Q10).

## Decisions

- **D1 — One sentence, one place.** `htui_core::qdrant::KNOWLEDGE_SEARCH_UNAVAILABLE =
  "knowledge search unavailable"` and `fn unavailable(cause: &str) -> String` (`"{SENTENCE}: {cause}"`).
  `htui-core` is the one crate `htui-mcp`, `htui-store` and `htui` all depend on. "Unavailable"
  covers: no URL, an unusable URL, the URL row unreadable or Postgres offline, the key unreadable,
  Qdrant unreachable, Qdrant refusing the key, an embedder mismatch, the model failing to fetch, load
  or embed the query (`R-STO-8`: "the Qdrant or the embedding model"). It does **not** cover: no
  collection (an empty index: "no matches — Ctrl+R indexes this scope", as today), no project in
  scope, an empty query.
- **D2 — The TUI and MCP search asks Qdrant first and never creates.** `QdrantIndex::search`
  (`concepts_worker.rs:573`) becomes, in order: settings (as today, microseconds when no URL); the
  cached connection when one exists for these settings; else `ExistingCollection::find_in` (no
  collection → `Ok(vec![])`, nothing created, no model loaded); then **the embedder check without the
  model** (D3); then the model (shared load, as today); then `found.open(embedder)` and the search.
  `sync` (Ctrl+R, the worker's index job, `--index-items`) keeps `QdrantStore::connect`, which creates
  by design.
- **D3 — Embedder check before the model.** `htui_store::model::identity()` is a free function
  (`model.rs:61`) and `DENSE_DIM` a constant, so `ExistingCollection` gains
  `check(&self, mine: &EmbedderIdentity, dim: usize) -> Result<(), StoreError>` reusing
  `check_existing` (`vector.rs:473`) read-only. A mismatched collection answers "embedder mismatch"
  without the 133 MB download. `open` keeps its own check (belt and braces, unchanged).
- **D4 — Two connection slots.** `Connections` caches what `connect` made (ensured: payload indexes,
  stamp, MOD-144 rebuild). A store opened read-only by D2 must never serve a later `sync`, which
  relies on `ensure_collection`. The slot records how the store was made (`Ensured` / `ReadOnly`);
  `search` takes either, `sync` takes only `Ensured` and otherwise connects (replacing the slot).
  Forget/clear rules unchanged (MOD-158 R1 B-L1).
- **D5 — `search_concepts` per session, from a cached flag (Q1, maintainer).** `ToolHost::open` is
  synchronous and runs on the store loop for chats (`R-NF-3`), so it cannot read the `qdrant_url` row.
  - `QdrantSource` keeps `configured: Mutex<Option<(bool, Instant)>>`: `true` exactly when the URL
    resolves (`resolve_url` Ok: the override or a usable row), `false` for no URL, an unusable one,
    a read error or offline; `None` until first known.
  - **Refreshed off the loop:** on every `publish` of a changed store (a spawned `url()` read); after
    every `settings()` resolution (each search, each index cycle); immediately after an in-process
    URL write or clear (`qdrant_settings_info` set/clear/adopt success); and by `offered()` itself,
    which spawns one background re-read (single-flight `AtomicBool`) when the value is older than
    **30 s**. `offered()` never awaits.
  - **Answer:** `offered()` returns the cached bool; unknown (`None`) answers `false` and triggers
    the read. The worker awaits one read at start (it already awaits `url()` there), so its first
    session knows; the TUI's first publish starts the read before any session can open.
  - **Seam:** `htui_mcp::search::ConceptSearch` gains `fn offered(&self) -> bool { true }` (default
    keeps test fakes unchanged); `McpHost::open` sets `caps.search = config.search.as_ref()
    .is_some_and(|s| s.offered())` (`host.rs:423`). `ConceptIndex` gains the same with default
    `true`; `QdrantIndex` answers from its source; `McpSearch` forwards.
  - **Worker:** the start-time gate (`worker_search`, `worker_cmd.rs:152`) goes; the worker always
    attaches `mcp_search::production()`, as the TUI does, and the flag decides per session. A URL set
    later is offered without a restart (supersedes MOD-158 Q3, as its decision record anticipated).
- **D6 — Tool error wording.** `search_concepts` answers `isError` with
  `unavailable(cause)` (was `search unavailable: {cause}`, `tools/search.rs:97-110`), for the
  no-index case too. During an outage on a configured workspace the tool stays offered (D5) and every
  call answers this.
- **D7 — The overlay (Q2, maintainer).** A failed search shows
  `knowledge search unavailable — <why-key> why` on the status row. A new rebindable action
  `Act::ConceptsWhy` (Concepts context, `capture_row`, default `ctrl-e` if the catalogue's conflict
  tests accept it in the Concepts stack — `ctrl-e` is today only `Form`'s `$EDITOR`,
  `catalogue.rs:1172` — else the blueprint picks a free chord) toggles one more row: the cause through
  the existing `with_place` (`concepts_search.rs:596`, e.g. "no Qdrant URL is set; set one in
  Settings > Qdrant…"). The hint row gains `why` only while there is a cause. The Ctrl+R index line
  keeps today's error rendering (`with_place`): an index run is not a search. Every key goes through
  `keys.toml` (no hardcoded key).
- **D8 — CLI wording.** `--search-items` keeps its exit codes and JSON `error.code`s
  (`state_cmd.rs:583-680`, MOD-92 contract). The human line and the JSON `error.message` for an
  unavailable search become `unavailable(<today's sentence>)`. `empty` stays `empty`.
  `--index-items` is unchanged (it is not a search). `docs/htui-cli.md` updated.
- **D9 — Warnings: what, where, never blocking (Q4 for the info line, maintainer).**
  - **One probe**, model-free: `htui_store::vector::probe(settings, collection) -> Probe` =
    `collection_exists` (+ `ExistingCollection::check`, D3, when it exists) with the key resolved as
    for a search. Outcomes: `Ready`, `NoCollection`, `Unreachable(sentence)`,
    `Refused(sentence)` (`is_qdrant_refused`), `Mismatch(sentence)`.
  - **Three warning states** (ANA-37 §8 Q6): **no Qdrant configured** (`Unresolved::NoUrl`, or an
    unusable URL); **unreachable or refusing the collection** (`Unreachable`, `Mismatch`, or
    `Refused` with a key present: "the Qdrant refused this box's API key"); **this box lacks the key**
    (`Refused` with no key from any source). Plus, by Q4, an **info line** (not a warning): a keyless
    probe that succeeds says "this Qdrant answers without an API key".
  - **Settings › Qdrant:** a warning row (theme's warning style) under the URL and key rows, from a
    probe the section requests when it reads its snapshot and on its reload key. The probe is a new
    `StoreRequest::ProbeQdrant` served **off the store loop** by `ConceptsRuntime` (beside
    `SearchConcepts`/`IndexConcepts`), never in `qdrant_settings_info::serve` (which runs on the
    loop). Offline and `--demo`: no probe; the URL row already says why.
  - **First run = TUI launch** (one Qdrant for the whole database since 2026-10-20, so a workspace
    switch changes nothing): once per launch, after the store is first online, the same probe;
    a warning state puts one status-line notice (`knowledge search unavailable: <state> — Settings ›
    Qdrant`), without clobbering an earlier notice, as MOD-158's old-URL launch check does
    (`app/state.rs:873`, `app/update.rs:488`). No modal, no block.
  - **Worker start:** after the publish, one `warn` line naming the state (or the info line at
    `info`); never an `error` (an `error` becomes a GlitchTip report, `docs/htui-worker.md`). The
    worker starts either way.
- **D10 — Gate step `qdrant` (Q3, maintainer).** Mirrors TOOL-14's `postgres` step:
  `HTUI_TEST_QDRANT_URL` unset → `skipped` (named in the PASSED line); set and its host:port
  unreachable within 3 s → `failed`; reachable → `passed` ("the Qdrant suites run live"). Placed after
  `postgres`. In lockstep: `scripts/gate.sh`, `scripts/gate.ps1` (`ValidateSet`, `$Steps`, self-test),
  the htui-dev mod's `GATE_STEPS` (`commands.ts:8`) and its test (`commands.test.ts:57, 128`),
  `docs/htui-dev.md:339`, README's test section. Self-test cases as for `postgres` (unset skipped,
  unreachable failed). Never print the URL beyond host:port.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Qdrant before the model | `concepts.rs:696-727` (`search_index`) | `ExistingCollection::find_in` → model → `open` → search |
| Settings resolution | `concepts_worker.rs:229-238` | `resolve_settings`, URL first so a box without one reads no key |
| Off-loop work | `concepts_worker.rs:815` (`ConceptsRuntime::serve`) | a request served on its own task, its reply sent by the task |
| Launch notice | `app/state.rs:873`, `app/update.rs:488` | `QdrantOldUrlCheck` dispatched at start, notice once, never clobbering |
| Gate step | `scripts/gate.sh:86-105` | `step_postgres`: 3 = skipped with a sentence, `/dev/tcp` probe, DSN never printed |
| Key row | `keys/catalogue.rs:1440-1463` | `capture_row(Act::ConceptsX, Concepts, "name", &["ctrl-x"], "help")` plus `position` and views list |
| Test seams | `concepts_worker.rs` tests, P-13 | `QdrantIndex::with_loader().with_source(…)` over a `QdrantEnv` literal; never `shared_qdrant` in a test |
| Live Qdrant tests | `htui-store/tests/qdrant_live.rs:30-60` | `HTUI_TEST_QDRANT_URL` or the skip line; throwaway collection; `HashEmbedder` |

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui-core/src/qdrant.rs` | UPDATE: D1 constant and `unavailable` | T1 |
| `crates/htui-store/src/vector.rs` | UPDATE: `ExistingCollection::check`, `probe`, `Probe` | T1 |
| `crates/htui-store/tests/qdrant_live.rs` | UPDATE: check/probe live cases | T1 |
| `crates/htui/src/concepts_worker.rs` | UPDATE: D2 search order, D4 slots, D5 flag, `ConceptIndex::offered` | T2 |
| `crates/htui/src/mcp_search.rs` | UPDATE: `offered` forward; `production` doc | T2 |
| `crates/htui-mcp/src/search.rs` | UPDATE: `ConceptSearch::offered` (default `true`) | T2 |
| `crates/htui-mcp/src/host.rs` | UPDATE: `caps.search` from `offered()` at `open` | T2 |
| `crates/htui-mcp/src/tools/search.rs` | UPDATE: D6 wording | T2 |
| `crates/htui-mcp/tests/tools_search.rs` | UPDATE: not offered when `offered()` is false; wording | T2 |
| `crates/htui/src/worker_cmd.rs` | UPDATE: drop the start gate; D9 start warning | T2 |
| `crates/htui/src/qdrant_settings_info.rs` | UPDATE: refresh the flag after a URL write (D5) | T2 |
| `docs/htui-worker.md` | UPDATE: `search_concepts` per session; start warning | T2 |
| `crates/htui/src/ui/overlay/concepts_search.rs` | UPDATE: D7 | T3 |
| `crates/htui/src/keys/catalogue.rs`, `crates/htui/src/keys/views.rs` | UPDATE: `Act::ConceptsWhy` | T3 |
| `crates/htui/tests/concepts_search.rs`, overlay snapshots | UPDATE | T3 |
| `docs/` key reference (the file listing Concepts keys) | UPDATE | T3 |
| `crates/htui/src/store_worker.rs` | UPDATE: `StoreRequest::ProbeQdrant`, reply, routing | T4 |
| `crates/htui/src/concepts_worker.rs` | UPDATE: serve `ProbeQdrant` off-loop | T4 |
| `crates/htui/src/ui/tabs/settings/qdrant.rs` | UPDATE: warning/info rows, probe request | T4 |
| `crates/htui/src/app/state.rs`, `crates/htui/src/app/update.rs` | UPDATE: launch probe and notice | T4 |
| `crates/htui/tests/qdrant_settings.rs`, Settings snapshots | UPDATE | T4 |
| `crates/htui/src/concepts.rs`, `crates/htui/src/state_cmd.rs` | UPDATE: D8 | T5 |
| `crates/htui/tests/state_cli_pg.rs`, `docs/htui-cli.md` | UPDATE: D8 | T5 |
| `scripts/gate.sh`, `scripts/gate.ps1` | UPDATE: D10 | T6 |
| `.claude/skills/htui-dev/hooks/commands.ts`, `.claude/skills/htui-dev/tests/commands.test.ts` | UPDATE: D10 | T6 |
| `docs/htui-dev.md`, `README.md` | UPDATE: D10 | T6 |

## Tasks

Order: **T1** first (the shared primitives). Then **T2, T3, T5, T6 in parallel** (disjoint file sets,
verified below). **T4 after T2** (both touch `concepts_worker.rs`; T4 serves the probe beside T2's
search). TDD throughout: failing test first.

### T1: Store and core primitives
- **Action**: D1 constant and `unavailable`; D3 `ExistingCollection::check`; D9 `probe` and `Probe`.
- **Tests**: unit tests for `unavailable`; `check` against `check_existing`'s cases (other width,
  other embedder, unstamped, older payload stamp accepted read-only); live (`qdrant_live`): `probe`
  on a missing collection (`NoCollection`, nothing created), an existing one (`Ready`), a
  mismatched one (`Mismatch`, no model), an unreachable URL (`Unreachable`).
- **Validate**: `cargo test -p htui-core`, `cargo test -p htui-store --all-features --test
  qdrant_live` (host owes the live run, see Risks).

### T2: Search path, session gate, worker
- **Action**: D2, D4, D5, D6; the worker half of D9.
- **Tests (no live Qdrant)**: `QdrantIndex` with a counting loader over an unreachable URL
  (`http://127.0.0.1:1`): a search fails with no model load (load count 0); no URL: count 0 (exists
  today, kept); `sync` after a read-only search connects (D4, via a fake or the slot's tag);
  `offered()`: unknown → false and one refresh spawned; URL set → true after the refresh; no URL →
  false; stale after 30 s re-reads once (paused time, no child process — see memory "paused time +
  real child"); `McpHost::open` with an `offered() == false` search does not advertise
  `search_concepts` and refuses a call to it as unknown; the tool's error text starts with
  `knowledge search unavailable: `. Live (`qdrant_worker` or a new case): a TUI search against an
  empty Qdrant creates no collection.
- **Validate**: `cargo test -p htui-mcp --all-features`; `cargo test -p htui --all-features --lib
  concepts_worker mcp_search worker_cmd`.

### T3: Overlay
- **Action**: D7.
- **Tests**: a failed `ConceptsReply::Hits` shows the sentence and the why hint; the why key toggles
  the cause row with its place; rebinding the key in `keys.toml` moves the hint; an empty index still
  shows `no_matches`; snapshots regenerated and each changed one read.
- **Validate**: `cargo test -p htui --all-features --test concepts_search --test keys`; overlay unit
  tests; `cargo insta test -p htui` reviewed by hand (memory: snapshot impact needs a full insta run).

### T4: Settings and launch warnings
- **Action**: D9 (Settings, launch). Depends on T1 (`probe`) and T2 (`concepts_worker.rs` layout).
- **Tests**: `ProbeQdrant` is served off-loop (the loop answers another request while a probe to a
  blackhole address is pending); each state renders its row (no URL, unreachable, mismatch, lacks
  the key, key refused, info line); offline and `--demo` send no probe; the launch notice appears
  once and never clobbers an earlier one; no key is ever rendered.
- **Validate**: `cargo test -p htui --all-features --test qdrant_settings --test qdrant_settings_pg`;
  Settings snapshots reviewed.

### T5: CLI wording
- **Action**: D8.
- **Tests**: `search_answer_with` over an unreachable URL: human line and JSON `message` start with
  `knowledge search unavailable: `, exit and `code` unchanged; `empty` unchanged.
- **Validate**: `cargo test -p htui --all-features --lib concepts state_cmd`;
  `cargo test -p htui --all-features --test state_cli_pg` (Postgres).

### T6: Gate step
- **Action**: D10.
- **Tests**: `bash scripts/gate.sh --self-test`; `pwsh -NoProfile -File scripts/gate.ps1 -SelfTest`
  (pwsh from /tmp per memory; a Linux run is not a Windows run); `bun test` / the mod's test runner
  for `commands.test.ts`; `bash scripts/gate.sh --only qdrant` with the sandbox's
  `HTUI_TEST_QDRANT_URL` (passed) and unset (skipped).
- **Validate**: as above.

## Validation

```bash
bash scripts/gate.sh            # fmt, clippy both ways, postgres, qdrant (new), test, abort-grep, …
cargo clippy --workspace -- -D warnings   # featureless (memory: featureless clippy gate)
cargo test -p htui --all-features -- --test-threads=1   # suite green is scheduling-dependent
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Live Qdrant suites time out in this sandbox (3.5-19 s creates, memory) | High | Unit tests carry the logic (unreachable URL, counting loader); live cases run, and are owed on the host if they time out; the done-report says which |
| A read-only store leaks into `sync` (no payload indexes, no MOD-144 rebuild) | Medium | D4's tagged slot, with a test |
| The cached flag lags a URL set on another box | Low (accepted, Q1) | 30 s re-read on session open; in-process writes refresh at once; documented |
| `offered()` false at the very first session after start | Low | Worker awaits the first read; TUI starts it at first publish; unknown is "not offered", matching `R-NF-2` |
| Probe blocks the store loop | Medium | Served by `ConceptsRuntime` (D9), tested with a blackhole address |
| `ctrl-e` conflicts in the Concepts stack | Low | Catalogue conflict tests decide; blueprint picks a free chord otherwise |
| Snapshot churn couples T3/T4 | Low | Disjoint views; T4 runs after T2, T3 in parallel touches overlay snapshots only |
| Gate step lists drift (4 copies) | Low | Self-test already compares `GATE_STEPS` to `STEPS`; the mod test pins it |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| TUI search loads the model before contacting Qdrant | true | `QdrantIndex::search` → `connect` → `embedder()` before `QdrantStore::connect` (`concepts_worker.rs:535-551, 573-582`) |
| TUI search creates the collection | true | `QdrantStore::connect` → `ensure_collection` → `create_collection` when absent (`vector.rs:747-758, 801-820`) |
| CLI asks Qdrant and the collection before the model, creates nothing | true | `search_index` (`concepts.rs:696-727`); `ExistingCollection::find_in` (`vector.rs:1034-1047`) |
| The embedder identity is available without loading the model | true | `model::identity()` free fn (`model.rs:61-70`); `RtenEmbedder::identity` just calls it (`embed.rs:297-299`) |
| `ToolHost::open` is synchronous | true | `fn open(&self, scope: ToolScope) -> Result<ToolLease, ToolHostError>` (`htui-orch/src/tools.rs:171`; `McpHost::open`, `host.rs:400`) |
| `search_concepts` advertised iff the host has a search attached at open | true | `caps: HostCaps { search: config.search.is_some() }` (`host.rs:~423`); `DEF.advertised: \|_, caps\| caps.search` (`tools/search.rs`) |
| TUI always attaches search | true | `mcp_search::production()` always `Some` (`mcp_search.rs:90-93`); `lib.rs:249-254` |
| Worker gates at start on `url()` | true | `worker_cmd.rs:117-133`, `worker_search` `:152-160` |
| Tool error today reads `search unavailable: <cause>` | true | `tools/search.rs:97-110` |
| Settings Qdrant requests are served on the store loop | true | `try_serve` routes `QdrantOldUrlCheck` and the Settings requests (`store_worker.rs:2384`) to `qdrant_settings_info::serve` |
| `ConceptsRuntime` serves concepts requests on tasks of their own | true | module doc and `REQUEST_NAMES` (`concepts_worker.rs:1-7, 37`) |
| A launch-time once-only Qdrant notice exists to mirror | true | `QdrantOldUrlCheck` dispatched in `App::start` (`app/state.rs:873`), absorbed in `update.rs:488` |
| The repo has no hosted CI config; the gate is the CI | true | no `.github/`, `.onedev-buildspec.yml` in `git ls-files`; `scripts/gate.sh` STEPS (`:44`) |
| Gate step list is pinned in four places | true | `gate.sh:44`, `gate.ps1:18, 35`, `commands.ts:8`, `commands.test.ts:57, 128`; `gate.sh` self-test compares `GATE_STEPS` (`:285-289`) |
| `qdrant_live` / `qdrant_worker` pass silently when `HTUI_TEST_QDRANT_URL` is unset | true | skip lines `qdrant_live.rs:32-38`, `qdrant_worker.rs:26-29` |
| `ctrl-e` is bound only in the Form context | true | `catalogue.rs:1167-1173` (`Act::FormExternalEditor`); the only other `ctrl-e` match is none; `ctrl-w` is global `Waiting` (`:1052`) |
| Concepts keys are `capture_row`s with ctrl chords | true | `catalogue.rs:1440-1463` |
| Task independence: T2, T3, T5, T6 file sets are disjoint | true | intersect the Files to Change rows: T2 {concepts_worker, mcp_search, htui-mcp search/host/tools/search + test, worker_cmd, qdrant_settings_info, htui-worker.md}; T3 {overlay, keys catalogue/views, concepts_search test, overlay snapshots, key doc}; T5 {concepts.rs, state_cmd.rs, state_cli_pg.rs, htui-cli.md}; T6 {gate.sh/ps1, commands.ts/test, htui-dev.md, README.md} — empty pairwise |
| T4 is not independent of T2 | true | both edit `concepts_worker.rs` → serial (T4 after T2) |

## Acceptance

- [ ] A TUI or MCP search against a dead or empty Qdrant loads no model and creates no collection
- [ ] Every search surface says "knowledge search unavailable"; the overlay's cause is one rebindable key away; CLI and MCP carry it after a colon
- [ ] `search_concepts` is offered per session only while a URL resolves; a URL set later is offered without a restart
- [ ] Settings, TUI launch and worker start warn (never block) on no URL, unreachable/refused, missing key; info line for a keyless Qdrant
- [ ] Gate has a `qdrant` step; the suites run live when `HTUI_TEST_QDRANT_URL` is set
- [ ] Gate green (live Qdrant suites on the host if the sandbox times out); reviewer gate passed

# Plan: MOD-66 — Per-box manual tool paths in Settings > Agents

**Source**: `HANDOFF.md` MOD-66 (from MOD-23, plan OQ-3; `R-AGT-6`; ANA-4 §4.6)
**Route**: plan (0 criteria fired; maintainer accepted 2026-10-01). Ultracode: not needed.
**Complexity**: Medium
**Decision numbering**: local, `D1`..: parallel sandbox runs share the global sequence, and a
local range cannot collide with theirs (the MOD-31 convention). Cited elsewhere as "MOD-66 D3".
**Status**: done (2026-10-01). Confirmed, implemented and reviewed; write-up `docs/decisions/mod/mod-66.md`.

## Summary

A human can give this box an explicit path for any `${tool}` an agent row's `discovery.tools`
declares. The path is stored in the row's own `agent_box.probe` document. The probe then uses it as
a resolution tier: the same probe runs over it (tier-2 handshake, status mapping), and the result is
recorded as `source: manual`. `recorded_launch` honours a manual snapshot, so the path reaches every
spawn. The entry point is a new form in the Settings > Agents section (`m`), beside MOD-23's
create/edit form.

## Where the item text was stale

- **`probe.tools` cannot carry a path.** It is `${name}` → *version*
  (`crates/htui-agent/src/probe.rs:1098`), so "writing `probe.tools` with `source: manual`" has
  nowhere to put one. The design below adds a field instead (D1).
- **"the driver resolves `agent.launch` against `probe.tools`"** is not what the tree does. A spawn
  either uses the recorded launch (`recorded_launch`, D58) or calls `tools::resolve`. That function
  walks the `HTUI_TOOL_<NAME>` env override (taken on trust), then PATH/npm/glob (`launch.rs:527`
  `launch_from` → `:552` `resolve_now`). `Agent::launch`'s own doc comment repeats the error
  (`crates/htui-core/src/model/agent.rs:42`); T1 corrects it.

## Design decisions

Maintainer answers, 2026-10-01: **manual tier in the probe** (not a hand-written `probe.resolved`),
and the **env override first**.

| ID | Decision | Why |
|---|---|---|
| D1 | `ProbeSnapshot` gains a trailing `manual: BTreeMap<String, String>` (tool name → absolute path), `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]`. It lives in the JSONB, so there is no migration, no `.sqlx` change and no store-trait change. | One row, one document. An old document reads as "no manual paths", and an empty map adds no key, so `a_probe_snapshot_orders_keys_as_ana4_does` (`tests/probe.rs:1972`) holds. |
| D2 | **Tier order per tool:** `HTUI_TOOL_<NAME>` (unchanged: checked with `exists`) → **manual path** → the tool's `discovery` probe. A manual path must be a **file** (`probe::is_file`, made `pub` and re-exported by T1). One that is not counts as missing for that tool (`warn!` naming the tool) and never falls through to discovery. | Maintainer: the env override stays the outermost escape hatch, and it agrees with the spawn-time fallback `tools::resolve`, which has no manual tier. `is_file` rather than `exists` because D58's disk check uses it: a directory is not spawnable. Falling through would silently replace the human's choice. |
| D3 | A manual resolution records **no version and checks no floor** (as the override does). For a `ToolProbe::Glob` tool it **keeps** the matching platform's `args` (unlike the override). | The human said which file is the tool. The glob's per-platform `args` (agy's `--uid=`) are a fact about the adapter, not the search, and dropping them is the dead adapter of MOD-2 blueprint H-4. |
| D4 | The manual map reaches the tool walk as an explicit parameter of the snapshot step. `probe_agent` takes it from `existing`'s snapshot, so every existing caller is unchanged: `probe_agents_on` `agent_worker.rs:2240` (`r` and the box probe), `run_reprobe` `:2574` (staleness and D60), login `:3102`, install `install/run.rs:116,160`. Every new snapshot **carries the map forward**, and `source` is `Manual` when at least one tool resolved through a manual path, else `Probe`. `probe_tools(discovery, env)` keeps its signature (10 call sites: `probe.rs:1417`, 8 in `tests/probe.rs`, `tests/agy_live.rs:161`) and delegates to a new `probe_tools_with(discovery, manual, env)`. | No caller change, and `source` stays an honest "who decided this launch". |
| D5 | D51's `Kept` rule is unchanged **for every probe path** (no `resolved` plus a stored `source: manual` writes nothing). It never applies to `SetToolPaths` (D9). | A manual path that vanished while nothing else resolves keeps the last good row and its map. |
| D6 | `recorded_launch` accepts `source` `Probe` **or** `Manual`; the status and `resolved` rules are unchanged, and D58's per-session disk check still applies. MOD-2 blueprint H-5 is superseded: a manual snapshot is now produced by the probe itself. The legacy hand-written shape (ANA-4's literal recipe) is honoured by the same rule. Docs reworded in T1: `ProbeSource::Manual` (`probe.rs:326`, "the editor is a later milestone's"), `recorded_launch`, `ProbeOutcome::Kept`. | This is the item's "resolution rule". `a_manual_snapshot_is_not_used_as_a_launch` (`tests/acp_driver.rs:782`) and the manual arm at `tests/probe.rs:2488` flip deliberately. Nothing else relies on the refusal. |
| D7 | New `StoreRequest::SetToolPaths { agent_id: AgentId, paths: BTreeMap<String, String> }`, `name()` = `set_tool_paths`. It is served by the **agent runtime** (it spawns tier 2) as a background task that answers itself. Answered with `StoreReply::AgentWritten { agents, outcome: AgentWrite::ToolPaths { id, name, status: ProbeStatus } }`. Routing goes in all three places: `try_serve` (`store_worker.rs:1475-1493`, compile-checked), the store-loop list (`:2129-2154`, wildcard), `crates/htui/src/testkit.rs:280-295` (wildcard). The "all sixteen" comment at `:1471` is updated. | A self-naming reply is what closes a Settings form (MOD-23 D240). The two wildcard lists fail silently if forgotten, so T2 adds a test that `SetToolPaths` reaches the agent runtime in both the shell and the testkit. |
| D8 | **Mutual exclusion, both directions.** `SetToolPaths` requires `claim_is_free` (box probe, install, login, writing background tasks) **and** `reprobe_claims.claim((agent_id, box_id))`, held for the whole task, as login does (`agent_worker.rs:1600-1608`; the D60 re-probe runs inside the chat task, where `claim_is_free` cannot see it). It runs as a `writes_agent_box` background entry, so install, login and box probe refuse while it runs. `probe()` (`ProbeAgents`, `:1287`) checks `claim_is_free` for no background writers today, so T3 adds an explicit refusal there while a tool-paths write is running. **Residual, named:** a D60 re-probe holds the `existing` row read at ChatStart; if that read predates a `SetToolPaths` write landing in between, the re-probe rewrites the row without the map. The window is a chat's length; the fix is a re-read in `run_reprobe`, which this item does **not** take on. It is noted in the write-up. | Without the reprobe claim, the two can write one row at once. Without the `probe()` check, only the section's UI `busy` guard stops `r`. |
| D9 | **Worker checks, in order and before any spawn:** a writer is present (offline: `REGISTRY_ON_SERVER_ONLY`); the box is registered; the agent exists and is `enabled`; its launch parses; every key is a tool `discovery.tools` declares; every value is absolute and `is_file`. The first failure answers `Failed { request: "set_tool_paths" }` with one sentence naming the tool. Then the handler probes with **`existing.source` forced to `Probe` in memory and the requested map**. A row with no `agent_box` row, or an unparseable `probe`, gets a synthesized minimal `existing` carrying the map. The blueprint may instead add a `pub` entry point taking the map directly and skipping `Kept`; either way the request **always writes** what the probe found. **Empty map = clear**: the probe walks discovery only and writes `source: probe`, `missing` included. | Without forcing `Probe`, a stored manual row plus a still-incomplete report would hit D51 and swallow the edit (fact-check amendment 7). A user who asked for a write gets one. Clearing needs no extra key. |
| D10 | **UI.** `m` on the highlighted row opens a "tool paths" form in the existing editor pane: one field per `discovery.tools` key, labelled with the tool name and prefilled from the stored map (an empty field means "no manual path"). Enter sends `SetToolPaths` with trimmed non-empty entries, Esc cancels, and an unchanged map closes the form with `UNCHANGED`. It is refused on a row with no `discovery.tools` ("this row's launch is literal; e edits its command") and while a write, probe, install or login is in flight (as `n`/`e`/`t` are). `on_written` (`agents.rs:1466`, exhaustive) gains the `ToolPaths` arm, which closes the form and notes the status word. `Failed{"set_tool_paths"}` clears `busy`: `REQUEST_NAMES` (`agent_settings.rs:480`) stays the store-loop list, and `agents.rs:2207` additionally accepts a new `agent_settings::SET_TOOL_PATHS` const. The `on this box` cell appends ` (manual)` for **every** status when `probe.source` is `manual` (`on_box_cell`'s early return at `agents.rs:1201-1203` is restructured). `HINT_IDLE` gains `m paths` (89 columns, under `NOTE_WIDTH` 98); the comment at `:109` and the free-keys comment at `:2068-2070` are updated. | `m` is unbound in Settings (`settings/mod.rs:333-352`); globally only Backlog binds it (`app/mod.rs:121`). `p` is the paste key while a login runs (`agents.rs:2125`). |
| D11 | **Labels and refusals are runtime strings here.** `Field.label` (`agents.rs:372`) and `Refusal.field` (`agent_settings.rs:84`) are `&'static str`. The tool-paths form uses its own field type with `String` labels (create/edit forms untouched). `parse_tool_path` returns a sentence that embeds the tool name, not a `Refusal`. Labels wider than `LABEL_WIDTH` 13 (e.g. `claude_agent_acp`, 16) size this form's label column to its longest tool name, capped at a third of the width. | The create/edit forms must not change behaviour or snapshots beyond the hint. |
| D12 | **Not changed:** `tools::resolve` and `resolve_now` (the spawn-time fallback gets no manual tier, D2; this is documented on `launch_from`); the `ProbeStatus` mapping; `agent_box` columns, store traits, conformance cases, `.sqlx`; `htui-orch`; `R-AGT-5` (nothing branches on an agent name). | Scope. |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Override tier, checked | `crates/htui-agent/src/probe.rs:430-465` (`probe_tools`) | `env_override_key`, existence check, `warn!` naming the key, `report.missing.push` |
| Snapshot field, serde-tolerant | `probe.rs:1107-1108` (`credential`, `#[serde(default)]`) | additive optional field; old documents keep their meaning |
| A probe that answers its own request | `crates/htui/src/agent_worker.rs:1287-1350` (`probe`), `:2172-2186` (`run_probe`) | refusal order before spawn; background task; one reply at the request's address |
| Claim | `agent_worker.rs:1724` (`claim_is_free`), `:1600-1608` (login's `reprobe_claims.claim`) | one claim for box probe, install, login; the per-row re-probe claim |
| Single-row probe and write | `agent_worker.rs:2553-2595` (`run_reprobe`) | `probe_agent(&agent, box_id, existing, &ctx, &SpawnTier2)` then `upsert_agent_box` |
| Request variant docs | `crates/htui/src/store_worker.rs:274-292` (`EditAgent`, `SetAgentOnBox`) | variant doc names its reply and its offline answer |
| Write outcome | `crates/htui/src/agent_settings.rs:485-522` (`AgentWrite`) | ids, names and status only; no `launch`, no `env` (`R-SEC-2`) |
| Form under the table | `crates/htui/src/ui/tabs/settings/agents.rs:325-380` (`Mode`, `Editor`, `Target`, `Field`), `:2083-2100` (`n`/`e`/`t` arms), `:1466` (`on_written`) | busy refusal, `UNCHANGED`, self-naming reply closes |
| Tests | `crates/htui-agent/tests/probe.rs:2453` (pure rule test), `tests/acp_driver.rs:782` (the driver honours or refuses), `crates/htui/tests/settings.rs` + `tests/snapshots/settings__agents_*.snap` (insta), `IDLE_KEYS` `tests/settings.rs:3600` | tempdir boxes, `plain_file`, `snapshot(source, status, resolved)` helpers |

## Files to Change

| File | Task | Action | Why |
|---|---|---|---|
| `crates/htui-agent/src/probe.rs` | T1 | UPDATE | D1–D6; `is_file` made `pub` and re-exported |
| `crates/htui-agent/src/lib.rs` | T1 | UPDATE | re-export `is_file` (if not reached via `pub mod probe`) |
| `crates/htui-agent/tests/probe.rs` | T1 | UPDATE | new cases; rule flip; literal at `:2426` |
| `crates/htui-agent/tests/acp_driver.rs` | T1 | UPDATE | flip `:782`; literal at `:529` |
| `crates/htui-agent/tests/auth.rs` | T1 | UPDATE | `ProbeSnapshot` literal at `:1218` |
| `crates/htui/tests/auth.rs` | T1 | UPDATE | literal at `:205` |
| `crates/htui/src/agent_worker.rs` | T1, then T3 | UPDATE | T1: literal at `:7241`; T3: handler, D8 exclusion, tests |
| `crates/htui/tests/settings.rs` | T1, then T4 | UPDATE | T1: literal at `:1909`; T4: form tests, `IDLE_KEYS` |
| `crates/htui-core/src/model/agent.rs` | T1 | UPDATE | doc fix at `:42` |
| `crates/htui/src/store_worker.rs` | T2 | UPDATE | variant, `name()`, `try_serve` + store-loop routing, comment at `:1471`, routing test |
| `crates/htui/src/testkit.rs` | T2 | UPDATE | route `SetToolPaths` to the agent runtime (`:280-295`) |
| `crates/htui/src/agent_settings.rs` | T2 | UPDATE | `AgentWrite::ToolPaths`, `SET_TOOL_PATHS`, `parse_tool_path` |
| `crates/htui/src/ui/tabs/settings/agents.rs` | T2 (stub arm), then T4 | UPDATE | T2: a compile-only `ToolPaths` arm in `on_written`; T4: D10/D11 |
| `crates/htui/tests/snapshots/settings__agents_{switched_off,unknown_row,empty,probed,demo,quota}.snap` | T4 | UPDATE | hint only |
| `crates/htui/tests/snapshots/probe__agents_probed_missing.snap` | T4 | UPDATE | hint only (`--test probe` binary) |
| `crates/htui/tests/snapshots/settings__agents_tool_paths_form.snap` | T4 | CREATE | the new form |

## Tasks

TDD per repo convention: each task's tests are written first and fail for the right reason.

### T1 — Probe: manual tier and resolution rule (`htui-agent`, plus literal fix-ups)
- **Action**: D1–D6, the `agent.rs:42` doc fix, and `manual: BTreeMap::new()` (or `..`) at all 9
  struct-literal sites without `..` (4 in `probe.rs`, plus the 5 test and `cfg(test)` sites listed
  above).
- **Tests first**: a manual path is used when PATH has nothing; the env override beats manual; a
  missing or directory manual path is missing (no fall-through); a glob tool keeps its platform
  `args`; the map is carried on a `Row`; `source` is `Manual` only when a manual path was used; D51
  `Kept` with a vanished manual path; `recorded_launch` accepts `Manual`; the driver spawns a manual
  snapshot; a document without `manual` round-trips byte for byte, and an empty map adds no key.
- **Validate**: `cargo test -p htui-agent --all-features -- --test-threads=1`;
  `cargo check --workspace --all-targets --all-features`

### T2 — Request and outcome types (`htui`)
- **Action**: D7's variant and the routing in all three places; `AgentWrite::ToolPaths`;
  `SET_TOOL_PATHS`; `parse_tool_path` (absolute, non-empty after trim, a sentence naming the tool);
  the compile-only `on_written` arm. Test: `SetToolPaths` is routed to the agent runtime in both the
  shell and the testkit.
- **Validate**: `cargo test -p htui --all-features --lib store_worker -- --test-threads=1`

### T3 — Agent runtime handler (`htui`), after T1 + T2
- **Action**: D8, D9. Mirror `probe` + `run_probe`/`run_reprobe`.
- **Tests first**: offline refuses before spawning; unregistered box; unknown agent or tool key;
  relative or non-file path; refused during a probe/install/login/box probe and while a re-probe of
  the row holds the claim; `ProbeAgents` refused while a tool-paths write runs; a valid map writes a
  `source: manual` snapshot carrying the map and answers `AgentWritten::ToolPaths`; a stored manual
  row plus an incomplete report still writes (no `Kept`); a row with no `agent_box` row; an empty
  map clears to `source: probe` and writes even `missing`; `user_off` still vetoes `enabled`.
- **Validate**: `cargo test -p htui --all-features --lib agent_worker -- --test-threads=1`

### T4 — Settings form (`htui`), after T1 + T2 (parallel with T3)
- **Action**: D10, D11.
- **Tests first**: `m` opens the form with one field per declared tool, prefilled; a literal row is
  refused; busy is refused; Enter sends `SetToolPaths` with trimmed non-empty entries; unchanged
  closes; a relative path is refused locally; `AgentWritten::ToolPaths` closes and notes the status;
  `Failed{"set_tool_paths"}` clears busy; the `(manual)` cell for `ready` and `missing`; snapshot
  `settings__agents_tool_paths_form`; `IDLE_KEYS` updated.
- **Validate**: `cargo test -p htui --all-features --test settings --test probe -- --test-threads=1`;
  review the 7 moved snapshots as hint-only diffs.

### Order and independence
**T1 ∥ T2 → T3 ∥ T4.**
- T1 = {probe.rs, lib.rs, htui-agent tests probe/acp_driver/auth, htui tests auth/settings,
  agent_worker.rs, agent.rs}.
- T2 = {store_worker.rs, testkit.rs, agent_settings.rs, agents.rs}.
- T1 ∩ T2 = ∅.
- T3 = {agent_worker.rs}; T4 = {agents.rs, tests/settings.rs, snapshots}; T3 ∩ T4 = ∅.
- T3 reuses T1's `agent_worker.rs`, and T4 reuses T1's `tests/settings.rs` and T2's `agents.rs`.
  Both run strictly after the first wave, so the overlaps are sequential, not parallel.

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features -- --test-threads=1   # Postgres suites run, not skipped
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The spawn-time fallback (`resolve_now`) ignores manual paths | Medium | D2/D12 accept it: a vanished recording usually means the manual path vanished too. Documented on `launch_from`. |
| A D60 re-probe overwrites a `SetToolPaths` write landing mid-chat | Low | D8 residual, named in the write-up; the fix (a re-read in `run_reprobe`) is out of scope. |
| A forgotten wildcard routing list sends `SetToolPaths` to the wrong worker | Medium | T2's routing test covers the shell and the testkit. |
| The `HINT_IDLE` change moves 7 snapshots and `IDLE_KEYS` | High | Expected; T4 reviews each as a hint-only diff. |
| A manual path to a `.cmd` shim on Windows | Low | Same as the env override today; MOD-16 owns Windows verification. Noted, not handled. |

## Verified claims

Fact-check, 2026-10-01, one agent over the tree (18 claim groups). Falsified or partial claims are
already amended above.

| Claim | Verdict | Evidence |
|---|---|---|
| `probe.tools` is name → version | TRUE | `probe.rs:1098` |
| Both drivers: `recorded_launch` → `launch_from` → `resolve_now` → `tools::resolve` (override on trust first) | TRUE | `acp/mod.rs:242-268`, `cli/mod.rs:282-292`, `launch.rs:527,552`, `tools.rs:11` |
| `Agent::launch` doc says "from `agent_box.probe.tools`" | TRUE (line 42, not 41-42) | `agent.rs:42` |
| `probe_tools` override: no version, empty args | TRUE; checks `exists`, not `is_file` | `probe.rs:430-465`, `:572` |
| `credential` is `#[serde(default)]`; adding a skip-if-empty field keeps documents | TRUE (pre-`credential` documents gain `credential:null` regardless) | `probe.rs:1107`; key-order test `tests/probe.rs:1972-2010` uses sequential `find` |
| `ProbeSnapshot` literal sites | 10, 9 without `..` (amended: T1 owns all) | `probe.rs:1365,1454,1467,1477`; `agent_worker.rs:7241`; `htui-agent/tests/probe.rs:2426`, `acp_driver.rs:529`, `auth.rs:1218`; `htui/tests/auth.rs:205`, `settings.rs:1909` |
| `probe_tools` call sites | 10, not 9 (`tests/agy_live.rs:161` added) | `probe.rs:1417`, `tests/probe.rs` ×8 |
| Every production `probe_agent` caller passes `existing` | TRUE | `agent_worker.rs:2240,2574,3102`; `install/run.rs:116,160` |
| D51 `Kept` rule | TRUE | `probe.rs:1341-1357` |
| `recorded_launch` refuses Manual; two pinning tests; nothing else relies on it | TRUE | `probe.rs:1174-1185`; `acp_driver.rs:782`; `tests/probe.rs:2488` |
| `claim_is_free` covers re-probes | PARTIAL: the D60 re-probe is invisible to it; `probe()` never checks writers (amended D8) | `agent_worker.rs:1724,1972,1600-1608,1287` |
| Routing for a new request | 3 places, 2 of them wildcards (amended D7, T2) | `store_worker.rs:1475-1493,2129-2154`; `testkit.rs:280-295` |
| `StoreRequest` variant count | 92 | `store_worker.rs:844` (`name()`) |
| `AgentWrite` matched exhaustively | `agents.rs:1466` (amended: T2 stub arm) | — |
| `Failed` clears `busy` only for `REQUEST_NAMES` | TRUE (amended D10) | `agents.rs:2207` |
| `Field.label` / `Refusal.field` are `&'static str` | TRUE (amended D11) | `agents.rs:372`, `agent_settings.rs:84` |
| `m` unbound in Settings; `p` bound only during a login | TRUE | `settings/mod.rs:333-352`, `app/mod.rs:121`, `agents.rs:2125` |
| Snapshots carrying `HINT_IDLE` | 7, including `probe__agents_probed_missing.snap`; plus `IDLE_KEYS` | `tests/settings.rs:3600,4788` |
| `probe::is_file` reachable from `htui` | FALSE: `pub(crate)` (amended: T1 makes it `pub`) | `probe.rs:589` |
| Probe types exported; `htui` depends on `htui-agent` | TRUE | `lib.rs:173-177`; `htui/Cargo.toml:28` |
| `on_box_cell` early-returns for non-ready | TRUE (amended D10: suffix for every status) | `agents.rs:1201-1203` |
| Tasks pairwise disjoint (original claim) | FALSE (amended: T1 ∥ T2 → T3 ∥ T4, sets above) | — |

## Acceptance
- [ ] A per-box manual path reaches a spawn (driver test) and survives every probe path
- [ ] The env override still wins; a missing manual path never falls through
- [ ] `SetToolPaths` always writes; clearing works from the form
- [ ] All tasks complete, gates green, patterns mirrored

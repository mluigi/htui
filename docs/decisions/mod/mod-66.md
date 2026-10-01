# MOD-66 - Per-box manual tool paths in Settings > Agents (done, 2026-10-01)

**Requirements:** `R-AGT-6` (manual entries allowed), `R-TUI-8` (Settings: agent registry),
`R-NF-3` (off the UI task), `R-SEC-2` (no `env` values in replies, notices or logs). `R-AGT-5` held
as a constraint: nothing added branches on an agent's name.
**Origin:** MOD-23 plan OQ-3 (D248), which deferred ANA-4 §4.6's per-box manual entry.
**Artifacts:**
- plan [`.claude/plans/mod-66-manual-tool-paths.plan.md`](../../../.claude/plans/mod-66-manual-tool-paths.plan.md): D1-D12, with its verified-claims table;
- blueprint `.claude/plans/mod-66-manual-tool-paths.blueprint.md`: B1-B18, hazards H-1-H-24, findings F-1-F-8;
- review `.claude/plans/mod-66-manual-tool-paths.review.md`.

Decision numbers are local to MOD-66 (the MOD-31 convention), because parallel sandbox runs share the global sequence.

Routed as **plan** (0 criteria fired). Run in a TOOL-7 sandbox (`hr/MOD-66`).

**Decisions (maintainer, 2026-10-01):**
- route accepted;
- design: a **manual tier inside the probe**, not a hand-written `probe.resolved`;
- precedence: the **`HTUI_TOOL_<NAME>` env override first**;
- plan confirmed;
- blueprint F-3: the marker is a short **`*`**, not ` (manual)`;
- review: **fix all** findings.

**Commits:**
- plan, fact-check and blueprint: `82597da`, `4b9efa0`, `653dee6`;
- T1: `ff25586`, `9d31b21`, `eee7e60`;
- T2: `84b3db1`, `901ac9f`, merged in `e424997`;
- T3: `eaa44fc`, `9bbb72b`;
- T4: `06000ca`, `ad2ab9f`, merged in `9c67714`;
- review: `9c23867` (findings), then `2b2f730`, `01278aa`, `af6cb59`, `5239c9b`, `d8db686`, `d363e87`, `6cb08b8`, `0bd2311`, `85b98c8`, `e873476` (fixes).

No migration, no `.sqlx` change, no store-trait change. `htui` gained `serde` as a direct
(workspace) dependency for review N5.

## Where the item text was stale

It said to write `probe.tools` with `source: manual`. `probe.tools` holds `${name}` → **version**,
so it has no place for a path. It also said the driver resolves `agent.launch` against
`probe.tools`. In fact a spawn either uses the recorded launch (`recorded_launch`, MOD-2 D58) or
calls `tools::resolve`, which walks the env override, then PATH, npm and glob. `Agent::launch`'s doc
comment and two `launch.rs` docs repeated the error; they are corrected.

## What shipped

**The probe's manual tier (T1, D1-D6).**
- `ProbeSnapshot` gains a trailing `manual: {tool: absolute path}`, `#[serde(default,
  skip_serializing_if = "BTreeMap::is_empty")]`, so every existing row keeps its bytes.
- The tool walk (`probe_tools_with`; `probe_tools` keeps its signature and delegates) is, per tool:
  1. `HTUI_TOOL_<NAME>`;
  2. the manual path, which must be absolute and a file;
  3. the row's `discovery` probe.

  A bad manual path is **missing**: it never falls through to discovery, which would silently swap
  the human's choice for the binary they wanted to avoid. A manual path records no version and
  checks no floor, as the override does. A glob tool **keeps** its platform `args` (the adapter's
  `--uid=`), unlike the override.
- `probe_agent` reads the map from the stored row and every snapshot carries it forward, so it
  survives `r`, the box probe, both chat re-probes and the install and login re-probes. None of
  those callers changed.
- `source` is `manual` exactly when a manual path decided a tool. MOD-2 D51's `Kept` rule is
  unchanged for those probe paths.
- **The resolution rule:** `recorded_launch` now accepts `source: manual`, so a manual snapshot
  reaches every spawn (D58's disk check still applies). This supersedes MOD-2 blueprint H-5.
  Consequence: ANA-4's hand-written recipe (a typed `resolved` with `source: manual`,
  `status: ready`) now spawns as written.

**The request (T2, T3, D7-D9).**
- `StoreRequest::SetToolPaths { agent_id, paths }` (now 93 variants) is served by the **agent
  runtime**, because it runs tier 2. It is routed in all three places: `try_serve`, the store-loop
  list and the testkit.
- It is answered with `StoreReply::AgentWritten` carrying `AgentWrite::ToolPaths { id, name,
  status }`. `SET_TOOL_PATHS` stays outside `REQUEST_NAMES`.
- Before any spawn the worker refuses, in order:
  - offline;
  - an unregistered box;
  - a gone or disabled agent;
  - a launch that does not parse (fixed sentence, no serde text);
  - a row with no declared tools;
  - a key that is not a declared tool;
  - a path that is empty, relative or not a file (the file check runs in the task, off the store
    loop).
- It then calls `probe_snapshot` (a `pub` entry point, never `Kept`) and **always writes** what
  the probe found. An **empty map clears** the entry and writes `source: probe`, `missing`
  included.
- **Exclusion:** the request holds the box claim (a `Writes::ToolPaths` background tag that
  `claim_is_free` and `ProbeAgents` refuse on, with their own sentence) and the row's
  `ReprobeClaims` claim for the task's lifetime.

**The form (T4, D10-D11).**
- In Settings > Agents, `m` on the highlighted row opens a tool-paths form under the table: one
  field per `discovery.tools` key, prefilled from the stored map, with an empty field meaning "no
  manual path". It uses its own field type, so tool names are runtime strings and MOD-23's forms are
  untouched.
- It is refused on a literal row, a launch that does not parse, and while a write, probe, install
  or login is in flight.
- A manual row's `on this box` cell ends in `*`, and the note line gains ` · * manual path` while
  one is visible.
- `HINT_IDLE` gains `m paths` (89 columns), which moved 7 snapshots (keys line only); 1 snapshot is
  new.

## Review round (`rust-reviewer`: approve with fixes, all fixed)

Two MEDIUM test gaps:
- **M1:** the row claim being held for the task's lifetime was not pinned.
- **M2:** the real handler's background tag was not pinned.

Both are now pinned by in-flight tests, each mutation-checked.

The LOW and NIT findings:
- **L1:** `is_file` moved off the store loop.
- **L2:** a macOS-portable backspace count.
- **L3:** an `R-SEC-2` sentinel test on the launch-parse refusal.
- **L4:** a verb-neutral claim sentence.
- **N1:** the note only while a `*` is visible.
- **N2:** the routing tests pin the handler.
- **N3:** `PathsForm`'s `Debug` prints counts.
- **N4:** the worker refuses literal rows.
- **N5:** two clones dropped.

Details and hashes: the review file.

## Known residue (named, not fixed)

- **D60 re-probe staleness (blueprint H-13).** A chat's end-of-chat re-probe uses the `agent_box`
  row read at `ChatStart`. If a `SetToolPaths` lands in between, the re-probe rewrites the row
  without the new map. The fix is a re-read in `run_reprobe`.
- **Spawn-time fallback (D12, H-14).** `tools::resolve` and `resolve_now` have no manual tier. A
  manual recording whose file is gone falls back to the override and discovery tiers. This is
  documented on `launch_from`.
- **A vanished manual path (D5, H-20).** Later probes `Kept` the last row (still `ready*`), and a
  chat then falls back and fails. `m` (re-save or clear) is the way out.
- **Older binaries (H-23).** An older `htui` on the same box drops the map on its next refreshing
  probe. Other boxes are unaffected.
- **Windows `.cmd` shims** in a manual path are as untested as in the env override (MOD-16).

## Gates

On the final tree, all green:
- `cargo fmt --check`;
- `cargo clippy --workspace --all-targets --all-features -D warnings`;
- `cargo doc -p htui-agent`;
- `cargo test --workspace --all-features -- --test-threads=1`: 3323 passed, 0 failed, 26 ignored. The baseline before
  MOD-66 was 3272. Postgres suites ran against the sandbox database.

The Postgres gate on the merged host tree runs on the host after `scripts/hr collect MOD-66`.

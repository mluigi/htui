# Plan: MOD-2 autodiscovery and box probe (milestone 5)

**Source PRD**: `.claude/prds/mod-2-agent-driver-chat.prd.md`
**Selected Milestone**: 5 (Autodiscovery and box probe). Milestones 1–2 landed under
`.claude/plans/mod-2-driver-seam-registry.plan.md` (`5c717d0`..`34d5f04`), milestone 3 under
`.claude/plans/mod-2-live-acp-chat.plan.md` (`a142fbf`..`682a423`), milestone 4 under
`.claude/plans/mod-2-durable-history-replay.plan.md` (`81d247b`). This plan continues their decision
(`D43`+) and task (`T22`+) numbering so a cross-reference never means two things. Milestones 6–9 are
out of scope; where one of them owns a seam this plan touches, the plan says where it stops.

**Design authority**: `docs/ANA-4.md` §4.6 (the two-tier probe, the probe table, what is written to
`agent_box`, the on-demand/on-registration timing, the Windows shim rule), §5.1 (`Discovery` /
`ToolProbe` / `VersionProbe` shapes, already shipped as types in milestone 2), §5.3 (the seed rows
the probe recipes live in), §9 build step 5 (`probe.rs`, `agent_box` writes, migration `0002`), §11
criteria 9 and 10. `docs/ANA-5.md` §9 (the five `COMMENT ON COLUMN` statements and the ten
`app_setting` keys that fold into `0002_agent_probe.sql`). `docs/ANA-2.md` §7 (MOD-4's skip
condition reads `agent_box.enabled` and `probe.status`), §9 (`0003_orchestration.sql` depends on this
file existing). `docs/REQUIREMENTS.md` `R-AGT-6`, `R-AGT-4`, `R-TUI-8`, `R-NF-3`.

**Requirements**: `R-AGT-6` (on box registration and on demand, probe `PATH` for known agent binaries
and ACP adapters, record version, mark enabled on that box; manual entries allowed), `R-AGT-4` (the
registry row is the box-independent recipe; per-box enablement lives in `agent_box`), `R-TUI-8` (the
Settings tab's agent registry section states what this box can run), `R-NF-3` (the probe spawns
processes and must never run on the UI thread or inside the worker's `select!` arm).

**Complexity**: Large

**Routing**: PRD path, continued. Routed as **plan** on 2026-09-08 with the maintainer accepting the
verdict (C2 and C4 fired; the PRD already exists and lists milestone 5 as its own row, so the chain
resumes at its second half). Ultracode accepted for the **implement** phase as one Workflow-tool
script; `code-architect` on Fable 5.1, implementers on Opus 5, the `rust-reviewer` gate on Fable 5.1.

## Summary

Four milestones have shipped a driver that can hold a live conversation, and every one of them has
had to be *told* what this box has: `tools::resolve` is a deliberately cheap resolver
(`crates/htui-agent/src/tools.rs:3`), `ToolProbe::Glob` resolves nowhere
(`tools.rs:195`, "glob probes arrive with MOD-2 milestone 5"), `AgentSummary.on_box` is `None` on
every row (`crates/htui-core/src/model/agent.rs:157`), and the Settings tab prints `not probed`
against every agent (`crates/htui/src/ui/tabs/settings/agents.rs:18`). This milestone makes the box
answer for itself.

It lands `crates/htui-agent/src/probe.rs`: tier 1 resolves each `agent.launch.discovery.tools` entry
(`path`, `node_package`, and — new here — `glob`) and captures its version through the recipe's own
`--version` args and regex; tier 2 spawns the resolved launch and completes an ACP `initialize`,
which is the only proof the binary actually runs. The outcome is a snapshot written to a new
`agent_box.probe JSONB` column plus the columnar `enabled`/`version`/`path`/`probed_at` fields that
have been in the schema since `0001`, through the `WriteStore::upsert_agent_box` method that has been
there since milestone 1. The Settings tab gains the trigger and the column; the migration
`0002_agent_probe.sql` carries ANA-4's column and ANA-5's five comments and ten settings keys, and its
existence is what unblocks MOD-4's `0003` (`docs/ANA-2.md:1848-1857`).

### Scope finding (decided here, surfaced at CONFIRM)

`docs/ANA-4.md:1268-1272` specifies the migration's second statement as:

```sql
COMMENT ON COLUMN agent.name IS NULL;  -- and fix the stale inline comment in 0001
```

That statement is a **no-op against this schema**. `0001_init.sql` carries no database comments at
all — `grep -c "COMMENT ON" crates/htui-store/migrations/0001_init.sql` is `0` — and the stale text
("`'claude','agy' seeded`", `crates/htui-store/migrations/0001_init.sql:96`) is a source-file `--`
comment, which no migration can reach and which the forward-only rule forbids editing in place
(`docs/ANA-9.md:349-350`). Setting a comment to `NULL` clears a comment that was never there.

D43 replaces it with a real `COMMENT ON COLUMN agent.name IS '<text>'` recording where the seed
actually happens, which is the intent ANA-4 stated. This is an **amendment to `docs/ANA-4.md` §9**,
recorded at close-out.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Migration | `crates/htui-store/cache_migrations/0002_agent_mirror.sql:1-19` | numbered sections with a header comment naming the ANA and the decision that authored them |
| JSONB column | `crates/htui-core/src/model/agent.rs:29-84` | the model holds `serde_json::Value` (`launch`, `settings`, `quota`); the typed shape lives in `htui-agent` (`launch.rs:59-193`) |
| Tool resolution | `crates/htui-agent/src/tools.rs:60-107` | tiered resolution with the `HTUI_TOOL_<NAME>` env override first, returning `DriverError::Unresolved` naming the first tool that resolves nowhere |
| Spawn | `crates/htui-agent/src/launch.rs:553-655` | `which` on `spawn_blocking`, then `spawn_supervised` (process group on unix, job object + `CREATE_NO_WINDOW` on Windows) |
| Handshake | `crates/htui-agent/src/acp/mod.rs:798-812` | one task owns the whole `connect_with` future; `block_task()` only at `initialize`; a failure carries `stderr_tail` through `handshake_error` |
| Store method | `crates/htui-store/src/pg/write.rs:416-442` | `upsert_agent_box` as `ON CONFLICT (agent_id, box_id) DO UPDATE` |
| Long op off the loop | `crates/htui/src/store_worker.rs:471-486` | a request intercepted before `try_serve`, answered by a spawned task that sends its own `ReplyEnvelope` once |
| Settings section | `crates/htui/src/ui/tabs/settings/agents.rs:56-84` | `wants_requests` + `on_reply` + `on_key`, one column per fact |
| Tests | `crates/htui-store/tests/migrations.rs:300-330`, `crates/htui-agent/tests/launch.rs` | migration assertions against a throwaway database; `#[ignore]` live tests for anything that spawns a real agent |

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D43 | **`0002_agent_probe.sql` is one file in three numbered sections**: (1) ANA-4's `ALTER TABLE agent_box ADD COLUMN probe JSONB;`; (2) ANA-5 §9's five `COMMENT ON COLUMN` statements verbatim (`prompt_template.body`, `prompt_template.name`, `run_step.prompt_digest`, `run_step.trim_record`, `step_graph_phase.token_budget`); (3) ANA-5 §9's ten `app_setting` rows as one `INSERT … ON CONFLICT (key) DO NOTHING`. ANA-4's `COMMENT ON COLUMN agent.name IS NULL` becomes a **sixth real comment** naming `htui_core::model::agent::seed_rows` / `PgStore::seed_if_empty_as` as the seeder. | The two ANAs both reserved this file (`docs/ANA-4.md:1267`, `docs/ANA-5.md:2143-2146`) and ANA-5's own reasoning for folding in is "MOD-2 authors `0002` and MOD-2 is this document's consumer, so there is zero sequencing risk". The `agent.name` amendment is the Scope finding above: a `NULL` comment clears nothing, and the reader ANA-4 wanted to warn is reading `psql \d+`, not the migration file. |
| D44 | **`agent_box.probe` is `Option<serde_json::Value>` on `htui_core::model::AgentBox`; the typed shape is `htui_agent::probe::ProbeSnapshot`.** `htui-store` and `htui-core` never parse it. | Exactly the split `agent.launch`/`agent.settings`/`agent_box.quota` already have (`agent.rs:29-84` holds `Value`; `launch.rs:59-193` holds the types). `htui-store` must not depend on `htui-agent`, and MOD-4's skip condition is a SQL predicate over `probe->>'status'` (`docs/ANA-2.md:1609-1615`), not a Rust match. |
| D45 | **The snapshot is ANA-4 §4.6's example, keys and all**: `transport`, `resolved {command, args, env}`, `tools {name: version}`, `handshake {at, protocol_version, agent_name, agent_version, capabilities, auth_methods}`, `status` (`ready` \| `unauthenticated` \| `missing` \| `failed`), `stderr_tail`, plus one key ANA-4's example does not have: **`source` (`probe` \| `manual`)**. | Every consumer named in ANA-2 §7 and ANA-5 §4.2 reads only `status`, so the shape is free to be the documented one. `source` is the mechanical form of ANA-4:791-798's rule that "a manual entry is never overwritten by a probe that finds nothing" — without a recorded origin that rule is a guess about who wrote a row. Recorded as an ANA-4 §4.6 amendment. |
| D46 | **One resolver, two callers.** `probe::resolve_tool(&ToolProbe, &Env) -> Option<ToolResolution { path, version, args }>` becomes the resolution core, gaining `Glob` and version capture; `tools::resolve` keeps its signature and its `HTUI_TOOL_<NAME>` first tier and delegates the rest. | `tools.rs:3` says the probe owns the tier it lacks, and its `Glob` arm is a stub that names this milestone (`tools.rs:195`). A second copy of the `PATH`/`node_modules` walk would drift, and the first thing it would drift on is the Windows shim rule. |
| D47 | **Version capture uses `regex` and `semver`, both already in the dependency graph** (`Cargo.lock`: `regex 1.13.1`, `semver 1.0.28`); they become direct workspace dependencies of `htui-agent`. A `VersionProbe.pattern` that fails to compile, or output that does not match, is **"present, version unknown"** — a resolution with `version: None`, never a probe failure. | The seeds hold real regexes (`"^v(\\d+\\.\\d+\\.\\d+)$"`, `crates/htui-core/seeds/agent_claude.json`) and a real semver floor (`node` `min: "22.0.0"`), so both are already the file format's vocabulary; `launch.rs:141` says the pattern is "held as text: milestone 5 compiles it". ANA-4:762 states the tolerance rule for `agy` explicitly. Neither crate adds a compiled crate to the tree. |
| D48 | **Glob matching is hand-rolled in `probe.rs`, no new crate.** `%VAR%` and `~` expand first (`dirs` is already a workspace dependency), then the pattern is split on `/` and walked one segment at a time, `*` matching within a segment only. Matches sort newest-`mtime` first so a JetBrains install with three versioned directories resolves to the newest. | The pattern set is closed and small (`crates/htui-core/seeds/agent_agy.json:19-28`: `*` per segment, nothing else), the repo's standing posture is not to add a crate for a closed grammar (ANA-5 §4.1 took the same call on templating), and the walk has to expand `%LOCALAPPDATA%` itself either way. |
| D49 | **Tier 2 is a handshake-only entry point in the ACP module**, `acp::handshake(io, settings, timeout) -> Result<Handshake>`: `connect_with` inside **one** task that owns the whole future, `InitializeRequest::new(ProtocolVersion::V1)` with the same `client_capabilities`/`client_info` the session path uses, then close and kill the child. No `session/new`, no prompt. | Milestone 3's reviewer CRITICAL was precisely "the SDK drops the foreground future when a connection actor fails, which orphaned the agent process" (`682a423`); a probe spawns *more* processes than a chat does, so it reuses the fixed shape rather than writing a second connect. Reusing `session_main` would mean a session, a `cwd` and a prompt the probe has no business inventing. |
| D50 | **Status mapping, in this order**: a required tool resolves nowhere → `missing`, **nothing is spawned**, `enabled = false`; spawn or `initialize` fails → `failed` with `stderr_tail` from `Spawned::stderr_tail()`, `enabled = false`; `initialize` succeeds with non-empty `authMethods` and no configured credential → `unauthenticated`, `enabled = false`; otherwise → `ready`, `enabled = true`. `discovery.handshake == false` stops at tier 1 and is `ready` on resolution alone. | `docs/ANA-4.md:1360-1364` states criteria 9 and 10 in exactly these words ("`probe.status = "missing"` and leaves `agent_box.enabled = false`"; "an unauthenticated box records `unauthenticated` rather than `ready`"), and ANA-4:711-713 gives the unauthenticated rule its reason: the vendor's own auth flow is not `htui`'s to drive. |
| D51 | **A probe never clobbers a manual entry.** When the stored row has `probe.source == "manual"` and this probe resolves nothing, the row is left exactly as it is (not even `probed_at` moves) and the outcome is reported to the caller as skipped. A manual entry that the probe *can* confirm is refreshed like any other. **Widened at the review gate (MEDIUM-3):** "resolved nothing" is `snapshot.resolved.is_none()`, not `status == Missing` alone — a `probe_tools` transport fault and an unparsable `agent.launch` also learn nothing about the box and must not clobber a hand-written row either. | `docs/ANA-4.md:795-798`. A `probed_at` bump on a no-op would make a hand-written row look freshly verified, which is the one reading the rule exists to prevent. |
| D52 | **The probe writes only where the registry is writable.** With no writable `Writer` (`Offline`, and `Buffered`, whose `upsert_agent_box` is `Unreachable`, `crates/htui-store/src/writer.rs:269-331`), the request is refused **before anything is spawned**, with that same message. | Probing costs process spawns; doing them to discard the result would be a lie told with a subprocess. Writing a probe result somewhere local is the local-only store's job (MOD-17), not this milestone's, and inventing a second sink here is what MOD-17's HANDOFF line forbids. |
| D53 | **`StoreRequest::ProbeAgents` is intercepted in the worker loop and served by a spawned task**, exactly as the four chat requests are (`store_worker.rs:471-486`): the loop reads `box_info()` and `agents()`, takes an owned `Writer` from `backend.writer()`, spawns the task with the request's `seq`/`origin`, and `continue`s. The task probes every enabled registry row, writes each `agent_box`, and sends one `ReplyEnvelope { reply: StoreReply::Agents(..) }`. | `R-NF-3`: a probe with a 60 s handshake timeout inside the `select!` arm freezes every other request, and inside the UI thread freezes the TUI. `Writer` is an owned handle implementing both `ReadStore` and `WriteStore` (`writer.rs:334`, `:392`), which is why the task needs no `Backend` (and `Backend` is not `Clone`). Answering with `Agents(..)` means the Settings section needs no second reply arm. |
| D54 | **The trigger is `r` in the Settings agent section**, whose `on_key` returns the request; while it is in flight the section renders `probing…` in the `on this box` column and refuses a second `r`. The column then reads `<version>`, `<version> (off)`, or the status word for anything not `ready`. | `R-TUI-8` and ANA-4:791's "`Settings > Refresh agents`". The section already owns its own keys and requests (`settings/mod.rs:42-58`), so this adds no keymap entry and no global action; `agents.rs:4` reserved this column for this milestone by name. |
| D55 | **Staleness re-probes lazily at `ChatStart`, tier 2 only, when `probed_at` is older than `PROBE_TTL` (24 h) or absent** — and a chat is never blocked by a probe *failure*: a stale-but-resolvable row still starts its session. Re-probe on spawn failure is **milestone 6's**, with the `cli` fallback that gives it somewhere to fall back to. | `docs/ANA-4.md:791-798` splits the triggers this way, and the reason it gives is that `agy` self-updates in place. Coupling chat startup to a probe verdict would make a 60 s handshake timeout a 60 s chat delay, so the probe informs the row and the chat proceeds on what resolution says. |
| D56 | **Criterion 10 (`agy`) is split.** The glob resolution, the per-platform `args` append (`["--uid="]` on Linux) and the `unauthenticated` mapping are proven here with temp-directory fixtures and a scripted handshake; the live `agy_acp_server` half needs a box with `agy` installed and lands in **milestone 6**, which is the `agy`-over-ACP milestone. The Windows glob (`%LOCALAPPDATA%\JetBrains\…`) and the `.cmd` shim message are runtime facts and join **MOD-16**. | Criterion 9 is fully provable on this box (`node` + `claude` + adapter are installed and milestone 3 already drove them live). Claiming criterion 10 from a Linux box with no `agy` would be the same false green TOOL-2 documents. |

## Files to Change

| File | Action | Why |
|---|---|---|
| `crates/htui-store/migrations/0002_agent_probe.sql` | CREATE | the `probe` column, six comments, ten settings keys (D43) |
| `crates/htui-store/tests/migrations.rs` | UPDATE | the migration applies, the column exists, ten keys land, a second run is a no-op |
| `crates/htui-core/src/model/agent.rs` | UPDATE | `AgentBox.probe: Option<Value>` (D44) |
| `crates/htui-core/src/store/mem.rs` | UPDATE | `upsert_agent_box` carries `probe`; the `agents()` join returns it |
| `crates/htui-core/src/store/conformance.rs` | UPDATE | `upsert_agent_box_by_pk` asserts `probe` round-trips and is overwritten on conflict |
| `crates/htui-store/src/pg/write.rs` | UPDATE | `upsert_agent_box` writes the column (D44) |
| `crates/htui-store/src/pg/read.rs` | UPDATE | `agents()` selects it into `AgentBox` (`read.rs:587`) |
| `crates/htui-store/tests/pg_criteria.rs` | UPDATE | column-value assertions for a written snapshot |
| `crates/htui-store/tests/writer_buffered.rs` | UPDATE | the `AgentBox` literal at `:261` gains the field; the refusal is unchanged (D52) |
| `crates/htui-store/.sqlx/` | UPDATE | regenerated after the `upsert_agent_box`/`agents` query changes |
| `Cargo.toml`, `crates/htui-agent/Cargo.toml` | UPDATE | `regex`, `semver` as direct dependencies (D47) |
| `crates/htui-agent/src/probe.rs` | CREATE | `ProbeSnapshot`, `ProbeStatus`, `resolve_tool`, glob walk, version capture, `probe_agent` (D45–D50) |
| `crates/htui-agent/src/tools.rs` | UPDATE | delegate to `probe::resolve_tool`; the `Glob` stub and its milestone-5 message go (D46) |
| `crates/htui-agent/src/acp/mod.rs` | UPDATE | `handshake` entry point beside `open_session` (D49) |
| `crates/htui-agent/src/lib.rs` | UPDATE | re-export `probe` |
| `crates/htui-agent/tests/probe.rs` | CREATE | tier 1 over temp dirs, version parsing, status mapping, manual-entry rule |
| `crates/htui-agent/tests/probe_live.rs` | CREATE | `#[ignore]` criterion 9: real `claude` adapter, `protocolVersion == 1` |
| `crates/htui/src/store_worker.rs` | UPDATE | `ProbeAgents` variant, `name()` arm, loop interception (D53) |
| `crates/htui/src/agent_worker.rs` | UPDATE | the probe task, and the `ChatStart` staleness re-probe (D53, D55) |
| `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | the `r` key, the in-flight state, the status column (D54) |
| `crates/htui/tests/settings.rs` | UPDATE | the key emits the request; the column renders each status |
| `crates/htui/tests/probe.rs` | CREATE | harness: the request is answered off the loop and lands as `Agents` |

## Tasks

Independence below is by **file set**, and the sets are listed for exactly that reason. Two tasks
whose sets intersect run serially even when their subjects look unrelated. Two chains run in
parallel: the store chain (T22 → T23) and the agent chain (T24 → T25 → T26). T27 needs both; T28
needs T27.

### T22: Migration `0002_agent_probe.sql`
- **Files**: `crates/htui-store/migrations/0002_agent_probe.sql`,
  `crates/htui-store/tests/migrations.rs`
- **Action**: write the three sections of D43 plus the sixth comment. Tests first: after
  `MIGRATOR.run`, `agent_box` has a `probe` column of type `jsonb`; the ten `app_setting` keys exist
  with ANA-5 §9's exact values; `col_description` is non-NULL for the six commented columns; running
  the migrator twice changes nothing. Values verbatim from `docs/ANA-5.md:2148-2189` — do not
  paraphrase the comment texts.
- **Mirror**: `cache_migrations/0002_agent_mirror.sql`'s header and numbered-section style.
- **Validate**: `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features`

### T23: `probe` through the three stores (serial after T22 — the column must exist)
- **Files**: `crates/htui-core/src/model/agent.rs`, `crates/htui-core/src/store/mem.rs`,
  `crates/htui-core/src/store/conformance.rs`, `crates/htui-store/src/pg/write.rs`,
  `crates/htui-store/src/pg/read.rs`, `crates/htui-store/tests/pg_criteria.rs`,
  `crates/htui-store/tests/writer_buffered.rs`, `crates/htui-store/.sqlx/`
- **Action**: add `AgentBox.probe: Option<Value>`; extend the conformance case
  `upsert_agent_box_by_pk` first (a snapshot round-trips; a second upsert replaces it; `None`
  clears), then make `MemStore` and `PgStore` pass it. Regenerate `.sqlx` from inside
  `crates/htui-store` per README.
- **Mirror**: `pg/write.rs:416-442`'s `ON CONFLICT … DO UPDATE`; `pg/read.rs:587`'s tuple-match
  construction.
- **Validate**: `cargo test -p htui-core --all-features` (**amended during implementation**: the
  bare `cargo test -p htui-core` compiles neither `store::conformance` nor
  `agents_join_this_box_only` — both sit behind `test-support`/`demo` — so it is a TOOL-2-shaped
  false green for this task);
  `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test -p htui-store --all-features`;
  `cd crates/htui-store && DATABASE_URL=…/htui_sqlx cargo sqlx prepare --check -- --all-targets --all-features`

### T24: Tier 1 — resolution, globs and versions
- **Files**: `crates/htui-agent/src/probe.rs`, `crates/htui-agent/src/tools.rs`,
  `crates/htui-agent/src/lib.rs`, `crates/htui-agent/tests/probe.rs`, `Cargo.toml`,
  `crates/htui-agent/Cargo.toml`
- **Action**: TDD `resolve_tool` over the three `ToolProbe` kinds: `Path` through `which` (the
  Windows shim rule of ANA-4:800-805 stays exactly as `launch.rs:553` has it), `NodePackage`
  local-then-global with the pinned fallback, `Glob` through the hand-rolled walker of D48 —
  `%VAR%`/`~` expansion, `*` per segment, newest mtime first, per-platform `args` appended. Version
  capture per D47, including the three seed patterns and the `node ≥ 22` floor, and "present, version
  unknown" for a non-matching line. Then make `tools::resolve` delegate and delete its `Glob` stub
  message.
- **Mirror**: `tools.rs:60-107`'s tier order and its `Unresolved` message; `launch.rs`'s
  `spawn_blocking` wrapper around `which`.
- **Validate**: `cargo test -p htui-agent --features test-support`; clippy over the crate;
  `cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features`

### T25: Tier 2 — the handshake-only ACP entry (serial after T24 — same crate, `lib.rs`)
- **Files**: `crates/htui-agent/src/acp/mod.rs`, `crates/htui-agent/src/probe.rs`,
  `crates/htui-agent/tests/probe.rs`
- **Action**: TDD `acp::handshake` against the recorded-transcript harness milestone 3 built: it
  completes `initialize`, reports `protocol_version`, `agent_name`, `agent_version`, capabilities and
  `authMethods`, and kills its child on every exit path (including the timeout). Then `probe_agent`
  composing tier 1 and tier 2 into a `ProbeSnapshot` with D50's status mapping and D51's
  manual-entry rule.
- **Mirror**: `acp/mod.rs:798-812`'s `block_task()`-only-at-initialize rule and `handshake_error`'s
  `stderr_tail` append; `open_session`'s timeout-then-abort shape.
- **Validate**: `cargo test -p htui-agent --features test-support`; clippy over the crate

### T26: Criterion 9 live (serial after T25 — same crate)
- **Files**: `crates/htui-agent/tests/probe_live.rs`
- **Action**: `#[ignore]` test, run explicitly: the seeded `claude` row probes on this box to
  `status = "ready"` with a `handshake.protocol_version == 1`; with `HTUI_TOOL_NODE` pointed at a
  nonexistent path the same row probes to `status = "missing"` and `enabled = false`, and **tier 2
  is never reached** — no adapter is spawned (D50). **Amended during implementation** (review gate,
  MEDIUM-4): the original wording said "no process is spawned", which is false with `versions: true`
  — tier 1 still runs `claude --version`, `npx --version` and `npm root -g`. What D50 promises, and
  what the test proves with a `Tier2` that panics if called, is that nothing is spawned *after* a
  required tool resolves nowhere.
- **Mirror**: `crates/htui-agent/tests/acp_live.rs`'s ignore-by-default convention and its run line.
- **Validate**: `cargo test -p htui-agent --features test-support --test probe_live -- --ignored --nocapture`

### T27: The request, the task and the trigger (serial after T23 and T25)
- **Files**: `crates/htui/src/store_worker.rs`, `crates/htui/src/agent_worker.rs`,
  `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/settings.rs`,
  `crates/htui/tests/probe.rs`
- **Action**: add `StoreRequest::ProbeAgents` with its `name()` arm and the loop interception of
  D53; the spawned task probing each enabled row, writing `agent_box` through the owned `Writer`, and
  answering once with `StoreReply::Agents`. Refuse with the `Writer`-less message before spawning
  anything (D52). Settings: the `r` key, the in-flight `probing…` state, the status column.
  Harness tests: the request is answered exactly once and addressed to the section's origin; an
  offline backend refuses it and spawns nothing.
- **Mirror**: `store_worker.rs:471-486`'s interception + `continue`; `agents.rs:96-131`'s column
  table.
- **Validate**: `cargo test -p htui --features testkit`

### T28: Staleness re-probe at `ChatStart` (serial after T27 — touches `agent_worker.rs`)
- **Files**: `crates/htui/src/agent_worker.rs`, `crates/htui/tests/chat.rs`
- **Action**: `PROBE_TTL` (24 h); a `ChatStart` whose `agent_box` row is absent or older than it
  runs tier 2 in the background and updates the row, while the chat starts on what tier 1 resolution
  already gave it (D55). Test: a stale row is refreshed after a chat starts; a probe failure does not
  fail the chat.
- **Mirror**: `agent_worker.rs`'s existing `ChatStart` flow.
- **Validate**: `cargo test -p htui --features testkit`;
  `USERNAME=htui-ci HTUI_TEST_DATABASE_URL=… cargo test --workspace --all-features`

## Validation

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test -p htui-store --all-features
cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features -- -D warnings

# after T23, from inside the crate (README "Postgres queries are checked at compile time")
cd crates/htui-store
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx migrate run --source migrations
DATABASE_URL=postgres://postgres:htui@localhost:5439/htui_sqlx cargo sqlx prepare -- --all-targets --all-features

# T26, explicitly (spawns the real adapter)
cargo test -p htui-agent --features test-support --test probe_live -- --ignored --nocapture
```

`USERNAME=htui-ci` is TOOL-2's standing workaround; a Postgres suite run without
`HTUI_TEST_DATABASE_URL` reports `ok` while proving nothing, so the Postgres line above is the one
that counts for T22, T23 and T28.

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The hand-rolled glob walker mishandles a pattern the seeds hold | Medium | D48 keeps the grammar to `*`-per-segment; T24 tests every seed pattern against a built temp tree, including the two-`*` JetBrains shape |
| A probe leaves an orphaned agent process | Medium | D49 reuses milestone 3's fixed connect shape; T25 asserts the child is killed on success, on failure and on timeout; the Windows job-object half is MOD-16's |
| A 60 s handshake timeout stalls the UI or the worker loop | Medium | D53 spawns the task; T27's harness test asserts the loop answers other requests while a probe is in flight |
| Probing overwrites a hand-written entry | Low | D51's `source` key and its test; ANA-4:795-798 is the rule being encoded |
| `0002` lands with ANA-5's sections wrong, and forward-only forbids fixing them in place | Low | T22 asserts each of the ten values and the six comment texts against `docs/ANA-5.md:2148-2189` |
| Criterion 10 claimed without an `agy` box | Certain if unguarded | D56 splits it; the HANDOFF phase note records the live half as milestone 6's |
| MOD-4's `0003` blocked longer than expected | Low | T22 is first and independent; the migration can land before the rest of the milestone if the maintainer wants MOD-4 unblocked early |

## Verified claims

Checked against the tree on 2026-09-08, before the CONFIRM gate. One claim was **falsified** and the
plan was amended before the maintainer saw it (D43, and the Scope finding above).

| Claim | Verdict | Evidence |
|---|---|---|
| **ANA-4 §9's `COMMENT ON COLUMN agent.name IS NULL` fixes the stale comment in `0001`** | **false** | `0001_init.sql` contains no `COMMENT ON` statement at all; the stale text is a source-file `--` comment at `crates/htui-store/migrations/0001_init.sql:96`, unreachable from SQL. D43 writes a real comment instead. |
| `agent_box` has no `probe` column today | true | `crates/htui-store/migrations/0001_init.sql:112-123` |
| `0002_agent_probe.sql` does not exist; `migrations/` holds only `0001_init.sql` | true | directory listing; the only `0002` is `cache_migrations/0002_agent_mirror.sql` |
| `cache_migrations/0002_agent_mirror.sql` names this file as milestone 5's | true | `cache_migrations/0002_agent_mirror.sql:9-11` |
| ANA-5 §9's additions are five `COMMENT ON COLUMN` + ten `app_setting` keys, folded into this file | true | `docs/ANA-5.md:2143-2189` |
| MOD-4's `0003` may not be applied before this file lands | true | `docs/ANA-2.md:1848-1857`, restated as risk 1 at `:2064` |
| MOD-4's skip condition reads `agent_box.enabled` and `probe.status` | true | `docs/ANA-2.md:1609-1615` |
| ANA-5's box profile projection does **not** read `probe` | true | `docs/ANA-5.md:510-513` ("No tags, no settings, no tool paths") |
| `WriteStore::upsert_agent_box` already exists on every store | true | `crates/htui-core/src/store/traits.rs:117`; `pg/write.rs:416`; `mem.rs:826`; `writer.rs:269` |
| `Writer` is an owned handle implementing both `ReadStore` and `WriteStore`, and `Backend::writer()` returns it | true | `crates/htui-store/src/writer.rs:45`, `:334`, `:392`; `crates/htui-store/src/backend.rs:138` |
| `Backend` is not `Clone`, so a spawned task cannot hold one | true | no `derive(Clone)`/`impl Clone` in `crates/htui-store/src/backend.rs` |
| `Writer::Buffered` refuses registry writes with a stated message | true | `crates/htui-store/src/writer.rs:269-275`, `:328-331` |
| `BoxInfo` carries `box_id`, so the task can address `agent_box` | true | `crates/htui-core/src/model/box_.rs:85-92`; `backend.rs:244` |
| The worker loop already intercepts requests before `try_serve` and lets a spawned task answer once | true | `crates/htui/src/store_worker.rs:471-486`, `:499-503` |
| `ToolProbe::Glob` currently resolves to `None` and names this milestone | true | `crates/htui-agent/src/tools.rs:97`, `:195` |
| `Discovery`/`ToolProbe`/`VersionProbe`/`PlatformGlob` types already exist | true | `crates/htui-agent/src/launch.rs:86-170` |
| The seeds hold regex version patterns, a `min` semver, and glob patterns with `%LOCALAPPDATA%`/`~`/`*` | true | `crates/htui-core/seeds/agent_claude.json`; `crates/htui-core/seeds/agent_agy.json:13-30` |
| `regex` and `semver` are already in the dependency graph; `glob` is not | true | `Cargo.lock` (`regex 1.13.1`, `semver 1.0.28`; no `glob`) |
| `dirs` is already a workspace dependency (for `~` expansion) | true | `Cargo.toml:35` |
| `Spawned::stderr_tail()` exists and is documented as this milestone's `probe.stderr_tail` | true | `crates/htui-agent/src/launch.rs:469-479` |
| The ACP path completes `initialize` with `ProtocolVersion::V1` inside one task, `block_task()` only there | true | `crates/htui-agent/src/acp/mod.rs:798-812` |
| The Settings agents section owns its own keys and requests (no keymap entry needed) | true | `crates/htui/src/ui/tabs/settings/mod.rs:42-58`; `agents.rs:56-84` |
| `AgentSummary.on_box` is `None` everywhere until this milestone | true | `crates/htui-core/src/model/agent.rs:157`; `crates/htui-store/tests/migrations.rs:322` |
| `AgentBox` literals needing the new field: 5 sites | true | `pg/read.rs:587`, `conformance.rs:1256`,`:1273`,`:1284`, `mem.rs:1210`, `tests/writer_buffered.rs:261` |
| Task independence: (T22→T23) ∩ (T24→T25→T26) = ∅; T27 intersects neither chain's files but needs both landed; T28 ∩ T27 = `agent_worker.rs` (declared serial) | true | file sets listed per task above |

## Acceptance

- [ ] All tasks complete — T22–T28
- [ ] Validation passes, Postgres line included, `cargo sqlx prepare --check` clean
- [ ] `docs/ANA-4.md` §11 criterion 9 demonstrated live on this box; criterion 10's non-live half
      demonstrated by fixtures, its live half recorded as milestone 6's (D56)
- [ ] `R-AGT-6` holds: a box answers what it can run without configuration, and a manual entry
      survives a probe that finds nothing
- [ ] `rust-reviewer` gate run over the whole change set (Fable 5.1), findings applied or explicitly
      deferred with the maintainer
- [ ] ANA-4 §9 amendment (D43) and §4.6 amendment (D45's `source` key) recorded in the HANDOFF phase
      note at close-out
- [ ] Patterns mirrored, not reinvented

## Close-out

_(filled at close-out: commits, amendments made during implementation with their reasons, and what
was left to milestone 6 / MOD-16.)_

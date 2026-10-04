# MOD-11 - htui MCP server (done, 2026-10-04)

**Requirements:** `R-MCP-1`, `R-MCP-2` (without `spawn_subagent`, which is MOD-27's), `R-MCP-3`,
`R-MCP-4`. Constrained by `R-ENT-8` (no agent moves a status), `R-ENT-9` (links only via MCP and the
importer), `R-ID-7`/`R-SEC-3` (scrub on the host, fail closed), `R-STO-8` (scoped search, clear
failure), `R-NF-1` (Windows) and `R-NF-2` (no daemon).
**Origin:** ANA-2 (`docs/ANA-2.md` §4.2, §8, §9 row MOD-11, risk 11), ANA-5 §4.2 (the box profile
projection), ANA-16 §8 (reachability from a worker or container).
**Artifacts:**
- PRD [`.claude/prds/mod-11-mcp-server.prd.md`](../../../.claude/prds/mod-11-mcp-server.prd.md):
  five milestones, PRD OQ-1..OQ-7;
- plan [`.claude/plans/mod-11-mcp-server.plan.md`](../../../.claude/plans/mod-11-mcp-server.plan.md):
  D1-D20, invariants I-1..I-8, OQ-1..OQ-10, tasks T0-T10, verified-claims table;
- blueprint `.claude/plans/mod-11-mcp-server.blueprint.md`: findings F-1..F-23, decisions B-1..B-21,
  escalation E-1;
- user guide [`docs/htui-mcp.md`](../../htui-mcp.md); notes in `docs/htui-worker.md` and
  `docs/personas.md`.

Decision numbers are local to MOD-11 (the MOD-31 convention).

Routed as a **PRD** (C2, C3 and C4 fired), ultracode accepted for implement and review, run in a TOOL-7
sandbox on `hr/MOD-11`. The plan was fact-checked before CONFIRM: 39 claims, 22 TRUE, 16 PARTLY,
1 FALSE (workflow `wf_33a62d14-658`), every non-TRUE verdict amended in place. The FALSE one was that
some prompt already told the agent to write its document through a tool; none did, which is D19.

**Decisions (maintainer):** PRD gate 2026-10-03, PRD OQ-1..OQ-7 resolved; plan CONFIRMED 2026-10-03
with OQ-1..OQ-10 as recommended; review verdict approve-with-fixes, round R1 applied 2026-10-04.

**Commits** (`95adf87f`..`a67e0321`, 89 on `hr/MOD-11`, 82 without the merges):
- PRD `95adf87f`; plan `7b14e46b` (fact-checked); CONFIRMED `c4bf516c`; blueprint `24155c7b`,
  `1d3013ea`, `fe6d49d5`, `1c78d7a0`;
- **W0, T0** (serial) `SessionSpec.prompt` and bridge types: `6f7858df`, `5fbde93f`;
- **W1, T1 ∥ T2 ∥ T3** in worktrees, merged `6dc7b124` (T1 fenced store writes: `24c92430`,
  `6477361e`, `fa0ba114`, `7978084f`, `75c06e3f`, `ed5e5570`), `ba4bce5d` (T2 `htui-mcp` crate:
  `dbc4d7a9`, `02db56dd`, `b9af24e7`, `c216361d`, `2de9b611`, `7633bce7`, `c9cedf94`, `4b7d366b`,
  `7d55ce80`), `f8cc2da9` (T3 transports: `f2eac726`, `ac234657`, `e0363eca`); wave gate G-W1
  4112 passed;
- **W2, T4 ∥ T5**, merged `fdd0985f` (T4 backlog tools: `3485b0cf`, `32736258`) and `d8c7a780` (T5
  `htui mcp` relay: `438b8c19`, `597f2607`, `2a32e4a2`, `b6a8fc76`);
- **W3, T6** (serial) engine registration and hosts: `955ee38d`, `301760d5`, `0f285a54`, `237ff1b0`,
  `51a16ff3`, `c4ec37f5`, `86f6307b`, `5076b1d3`, `7534df2b`, `9cfeba59`;
- **W4, T7 ∥ T8**, merged `4a921728` (T7 `search_concepts`: `2ec47473`, `def3d1a7`, `c4c30a89`) and
  `243644f0` (T8 `command_run`: `2b10e1d0`, `6114e510`, `3eb31473`, `46b63c36`, `66cbebca`,
  `a5900f70`, `3eb93450`, `9f20c3a0`, `cfa711f3`); then **T9** CLI permission prompt (`00937229`,
  `438f0c5d`, `dc0e896c`, `dc89dacc`, `3970c69d`, `f600b5ca`, `bfd84d68`, `0c1186a8`, `4c56a840`,
  `8e410ff1`) and **T10** docs (`62125bc9`, `3f2ae450`, `1465fd4b`, `d21a8d44`, `f0a48d28`,
  `45eb6f91`, `426ab8dd`, `852c5c45`);
- **review round R1**: `23a91668`, `9e0513a2`, `9ed58d5c`, `96868c87`, `4e9cbac5`, `be1305b0`,
  `29d1a6b4`, `8df63f37`, `a67e0321`;
- this close-out.

Migration `0014_command_queue`. New crate `crates/htui-mcp` (the seventh workspace member).

---

## What was built

Every agent session htui launches (engine steps in the TUI and in `htui worker`, fresh and promoted
chats) gets one MCP server named `htui`. The agent sees its tools as `mcp__htui__<tool>`.

| Tool | What it does | Offered when |
|---|---|---|
| `box_profile` | the box's `BoxProfile`, as JSON and as the prompt's `box` section renders it | always |
| `document_write` | writes the step's `output_kind` document on the run's item; each call a new version | the session has an item and an output kind |
| `note_add` | a note on the run's item, `via_step_id` set, at most 16 KiB after scrubbing | the session has an item |
| `item_status` | a *request*: a note `status request: <status> (<resolution>) — <reason>`; never a transition | the session has an item |
| `item_link` | `add` proposes (or revives) a link from the run's item to a key in the same project; `remove` tombstones a link this run proposed | the session has an item |
| `search_concepts` | the concepts index, restricted to the session's project; item and requirement hits | the host has a concepts index |
| `command_run` | `build`/`test`/`run` through a per-`(box, class)` queue; returns the scrubbed 64 KiB tail | the phase exposes the queue (below) |
| `permission_prompt` | the claude CLI's `--permission-prompt-tool` target | `claude-cli` sessions only |

**Scoping and the token (OQ-1, I-1, I-6).** The host mints a 256-bit token per session and registers a
`ToolScope` for it (run, step, item, project, box, user, `StepFence`, output kind, hostname line,
command-queue flag, cwd, transport). No tool takes a run, step, project or box id; argument objects
refuse unknown keys. The token is unregistered when the session ends, and later calls answer
`session ended`.

**Process boundary: a stdio front and a host listener.** The agent starts `<htui binary> mcp` with
`HTUI_MCP_ADDR` and `HTUI_MCP_TOKEN` (D6). The relay holds no DSN and parses no MCP: it handshakes
`{"token","version"}` and splices bytes. The binary is `/proc/<host pid>/exe` on Linux, so a binary
replaced on disk still matches. The **hosting** htui process answers the MCP: hand-rolled NDJSON
JSON-RPC 2.0 (OQ-2, no `rmcp`), protocol versions `2025-11-25`, `2025-06-18`, `2025-03-26`,
`2024-11-05`, 1 MiB line cap. Its listener is a Unix socket `s` in a fresh `0700` directory
`htui-mcp-<pid>-<8 hex>` under `$XDG_RUNTIME_DIR` or the temp dir, or a Windows named pipe
`\\.\pipe\htui-mcp-<32 hex>` (first instance, remote clients refused). It is bound lazily at the
first session and removed on shutdown. The relay exits 0 / 1 / 2 / 3 (`McpExit`, never reported to
Sentry).

**Wiring.** ACP gets a stdio entry in `session/new`'s `mcpServers` (D7). `claude-cli` gets one
`--mcp-config=<json>` argument with no `--strict-mcp-config` (D8). `htui-orch::tools::ToolHost` is the
seam (D4): `EngineParts.tools: Option<Arc<dyn ToolHost>>`, `None` keeps every spec and prompt byte
for byte as before. `drive_once` opens a `ToolLease` per session; the judge's two calls get two
tokens on one step. `htui_mcp::McpHost<H: WorkerHost>` implements it, with `set_host` (B-2) so the
TUI's swapped `Backend` is followed. The TUI shares one host between walks and chats; `htui worker`
builds its own and refuses to start without one.

**Store (D13, B-4, D14).** New `WriteStore` methods, all forwarded through `WorkerStore`, `Writer`,
`UsageSpy` and `SpyStore`:
- fenced item writes `write_step_document`, `add_step_note`, `propose_link`, `withdraw_link`, each
  refusing a step that writes on another run's item;
- the read `item_by_key(project, key)`;
- the queue `enqueue_command`, `claim_command`, `beat_command`, `finish_command`, `cancel_command`.

On Postgres the fenced writes go through `step_scope`, which locks `FOR SHARE OF s, r` (step → run →
item, `park_step`'s order). `write_document` stays unfenced for `close_out` and the test author.
`ProgressSink` now writes through `write_step_document(StepFence::Lease(owner))`. Migration `0014`
adds `command_run.claimed_by` and `heartbeat_at`. A `running` row unbeaten for 30 s and a `queued`
row whose waiter stopped asking are reaped by the next claim of the same `(box, class)`.

**`command_run` (OQ-7, D15).** Runs on the host's box through `sh -c` / `cmd /C` in the session cwd
(or a relative subdirectory). The timeout is 1-1800 s, and a kill reaches the process group or job
object (`htui_orch::verify::run_shell`, B-15). The tail is scrubbed fail-closed. Limits are
`app_setting.command_limits` overlaid by the box's own, with a missing class at 1. The worker's
`{"verify":1}` fallback is gone. `verify` stays the orchestrator's.

**Exposure and denials (D16 as amended, D17).** `command_queue_exposed(mode, fan_out, item_tags)`:
`off` false, `always` true, `fan_out_only` when `fan_out > 1` or the item's tags hold
`heavy_build`. The engine's `command_run_exposed` also requires a tool host (R1-C), and the persona
term is `persona_keeps_command_run` (R1-H1). The prompt's `command_queue` section and the tool read
the same value. When exposed, ACP steps get one `reject_once` rule per heavy prefix
(`HEAVY_COMMAND_PREFIXES`, whole-word match via `PermissionMatch.command_word`, R1-L4) at the head of
the policy, and `claude-cli` gets `Bash(<prefix>:*)` in `--disallowedTools`.

**Output instruction (D19, B-7).** When a session is offered `document_write`, its prompt ends with a
protected `<section name="output">`, one fixed sentence naming the tool. `Placeholder::Output` is
internal (not in `ALL`), so no template and no existing golden moved. There is no fallback: a session
that writes no document still fails `missing_output` / `MissingDocument`.

**CLI permission prompt (D18).** A `claude-cli` session with a prompt port runs with
`--permission-prompt-tool mcp__htui__permission_prompt`. Its calls become ordinary
`DriverEvent::PermissionRequest`s (`allow_once`/`reject_once`), so the MOD-42 relay and the chat loop
answer them unchanged. Caps stay per-row false. `SelectInput.inline_prompt` admits a CLI agent to a
gated phase when the engine has a tool host. A start-time guard fails a gated CLI step whose lease
came back without a port (`missing_capability: inline_approval`).

**Pre-approval (D20, R1-M2).** `htui_orch::tools::pre_approve` appends an `allow_once` rule per
advertised htui tool except `command_run` and `permission_prompt`, matched on
`mcp__htui__<tool>`. The rules come after the R-MCP-4, persona and agent rules, only under an `ask`
default (never widening a `deny` default, `8df63f37`). It runs in `drive_once` and on both chat paths.

## Why

No step could write its `output_kind` document outside a test, so every production judge failed and
parked the fan-out on a human pick (MOD-4 M4 OQ-4), and production `approve`/`accept` stayed greyed
(MOD-4 R-50). Agents could not leave attributed notes or links, and heavy commands ran unthrottled
through the agent's own shell. The host-side design keeps ANA-16's rule that agents never write
Postgres: the relay holds nothing, and the process that already owns the store, the lease and the
scrubber answers every call.

## Decisions worth keeping

**PRD questions (2026-10-03).**
- OQ-1: stdio front plus host relay, no loopback HTTP.
- OQ-2: CLI `permission_request` in scope (M5).
- OQ-3: `box_profile` honours the hostname switch (closes MOD-33 D277).
- OQ-4: `item_link` live on insert with its own tombstone.
- OQ-5: `document_write` only for the phase's `output_kind` on the run's own item.
- OQ-6: `command_run` output capped and scrubbed; `verify_command` stays direct.
- OQ-7: `requirement_cite` deferred.

**Plan questions (CONFIRMED as recommended).**
- OQ-1: a private local socket, never TCP, a scoped reading of ANA-16 §5.4.
- OQ-2: hand-rolled MCP.
- OQ-3: liveness columns in `0014`.
- OQ-4: every item write fenced.
- OQ-5: the R-MCP-4 prefix list is a constant.
- OQ-6: accept the `fan_out_only` digest churn.
- OQ-7: `command_run` through the platform shell on the host's box.
- OQ-8: chats get the server.
- OQ-9: an instruction in the prompt, no fallback document.
- OQ-10: a persona `deny` such as `mcp__*` may hide htui's tools, honoured and documented.

**Amendments made during the run.**
- **D13's lock order is step → run → item.** T1's first `step_fence`-then-item shape locked only the
  run and deadlocked against `park_step` (`FOR UPDATE OF s, r`) under the Pg race test (40P01).
  `step_scope` takes `FOR SHARE OF s, r`.
- **D16 amended by R1-H1:** `command_run = base && persona && !deny_kinds ∋ execute`. An
  execute-denying persona loses the tool, and the prompt term uses the same helper.
- **I-3 reads "every item write".** `command_run` is not fenced (`45eb6f91`); its lease handling is
  MOD-78.
- **D20 (new, R1-M2):** pre-approval of htui's own tools, as above.
- **`document_write` writes several versions per step** ("newest wins"), relaxing ANA-2 `:424-431`'s
  "exactly one per step", as MOD-4's judge already assumed.
- **`heavy_build` is also a required box tag** (`3f2ae450`): it lives in `item.required_tags`, so it
  both exposes `command_run` and restricts which boxes may run the item (R-ORCH-10). Whether that is
  wanted is ANA-28.

**Blueprint decisions worth keeping.**
- B-2: `McpHost::set_host` follows the TUI's backend; each session captures its writer at `open`.
- B-6: `propose_link` keeps a live link's proposer, so a run can withdraw only what it proposed or
  revived.
- B-7: the output section is internal.
- B-9: `htui_orch::fake::FakeToolHost` for engine tests; the production path is pinned in
  `run_worker.rs` and `runs_pg.rs`.
- B-10/B-11: protocol versions; progress notifications for `command_run`.
- B-12: the handshake version is `RELAY_VERSION` (`CARGO_PKG_VERSION+relay.1`).
- B-15: `run_shell` beside `ShellVerifier`.
- B-17: the `ToolLease` is a local declared before the session, so the agent drops first.
- B-19: `ToolHost::close`.
- B-20: `htui-mcp` spawns only through `htui_agent::contained` and ships its own `clippy.toml`.
- **E-1** (token in claude's argv) was built as the plan said; the hardening is MOD-79.

## Defects caught by the per-task verify lenses

Each task ran implement → conformance and adversarial read-only lenses → repair (at most two rounds).
- **T1:** the run-only lock deadlock above; NUL in agent text refused on both stores; link writes'
  fence pinned after a take.
- **T2:** a panicking tool call was never answered (now `-32603`); duplicate request ids; the Windows
  relay's half-close on stdin EOF; a broken pipe instance retried forever; connections now hold the
  host weakly.
- **T5:** the relay lingered 5 s after the host ended the session.
- **T6:**
  - the judge prompt carried two output instructions (the candidate's trailer is dropped from the
    judge's task);
  - the tool host went stale after an offline blip (it now keeps the last writable backend);
  - the shared host was closed before the chats.
- **T7:** the worker loaded the embedding model twice (one shared model now).
- **T8:** a queued row whose waiter died blocked its `(box, class)` forever (now reaped); a client
  that stopped reading froze `command_run`'s heartbeat.
- **T9:**
  - `answer_permission` deadlocked past 256 queued events (it drains while waiting);
  - a cancel wrote double answer rows;
  - a chat with a prompt port still showed the "cannot ask" banner;
  - a prompt was announced before its tool call.
- **T10:** doc corrections against the code, among them `heavy_build`'s box-tag effect, `command_run`
  being unfenced, `remove` of a non-live link answering `not found`, and listener failures failing
  steps rather than the worker's start.

## Review gate

`rust-reviewer`: approve-with-fixes. Each finding was verified adversarially (workflow
`wf_e92a313a-ab2`). Round R1 applied:
- **M2:** pre-approve htui's own tools (D20), and never under a `deny` default; the verify lens caught
  that widening (`8df63f37`).
- **H1:** an execute-denying persona loses `command_run` (amends D16).
- **C:** a host-less engine offers no `command_run`, so no denials and no section.
- **M1:** background children neither hold nor outlive `command_run`: a 2 s pipe grace after the
  shell exits, then a group kill. The orphan checks fail rather than pass vacuously (`a67e0321`).
- **L4:** R-MCP-4 denials match whole command words (opt-in `PermissionMatch.command_word`; user and
  persona rules keep raw prefixes).
- **L3:** one `command_run` at a time per session.
- **L1:** stale `dead_code` allows removed.

Refuted: **L2**. No current `Debug` path logs the token. The hardening is folded into MOD-79.

## What changed in other items

- **MOD-4:** R-50 and M4 OQ-4 are closed. Production judges resolve on agent-written `judge`
  documents with the production sink (`author: None`), and `approve`/`accept` are live on a step whose
  document came through `document_write`.
- **MOD-41 D5:** closed. The sink's document write is fenced.
- **MOD-33 D277:** settled. `box_profile` honours the project's hostname switch through the same
  `HostnameLine` as the prompt.
- **MOD-34 / MOD-50:** `search_concepts` exists, scoped to the session's project, with requirement
  hits.
- **MOD-2:** the declared CLI `permission_request` gap is closed whenever htui hosts the tools
  (ANA-4 `:1385-1386`'s open contract).
- **MOD-27:** the server and the `ToolHost` seam exist; `spawn_subagent` is one more file under
  `crates/htui-mcp/src/tools/`.
- **MOD-44:** a container agent must reach the host's socket (bind-mount) and start a relay it can
  see. `/proc/<pid>/exe` is not visible inside a sandbox that does not share `/proc`
  (`docs/htui-mcp.md`, Troubleshooting).

**Stale HANDOFF facts corrected by the PRD evidence.**
- The production sink was `ProgressSink { author: None }`, not `NoSink`.
- Document versions are allocated by the store (`max(version)+1` under the item lock), not by the
  orchestrator.
- `BoxProfile::project` lives in `htui_core::model::box_`, not `htui_core::prompt`.
- D277 is a MOD-33 plan decision (`.claude/plans/mod-33.plan.md`), carried in `mod-33.md`.

## Gate

Run on the post-review tree (code at `a67e0321`, 2026-10-04), serially with Postgres:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean |
| `cargo test --workspace --all-features --no-fail-fast -- --test-threads=1` | 4266 passed, 0 failed, 30 ignored; SIGABRT 0; no `htui mcp` process or socket directory left |
| `cargo sqlx prepare --check` (`crates/htui-store`, migrated scratch DB) | exit 0 |
| `cargo check -p htui-mcp --target x86_64-pc-windows-gnu` | clean (in this sandbox with `CC_x86_64_pc_windows_gnu=gcc AR_x86_64_pc_windows_gnu=ar`: no mingw) |
| pending snapshots | none |
| `validate-workflow-docs.sh` | green |

**Pins moved** (re-counted 2026-10-04):
- store conformance `CASES` 134 → 144 (T1 six, T8 four); `READ_CASES` 15 (HANDOFF said 14, stale
  since MOD-72); `htui-orch` `CASES` 92 → 98;
- `.sqlx` 322 → 341 files; `crates/htui/tests/snapshots` 143 (unchanged; `backlog__runs_reject_note`
  moved with OQ-6), plus two new `htui-core` `prompt_output__*` goldens;
- migrations 14 (next `0015`; cache still `0001`..`0004`); `TABLES` 42 (unchanged); pinned commented
  columns 44 → 46;
- workspace members 6 → 7; `StoreRequest`/`StoreReply` unchanged.

## Left open

Filed:
- **MOD-77**: Pg step writers lock the step before the run (`park_step`'s order). Not reachable
  through MOD-11.
- **MOD-78**: `command_run` lifecycle: a lease check and a cancellation signal on session end.
- **MOD-79**: the MCP token off claude's argv (E-1, review L2).
- **ANA-28**: `heavy_build` as a queue switch versus a required box capability.

Notes, not filed:
- The Windows named-pipe path is compiled (`x86_64-pc-windows-gnu`) but never exercised. Its runtime
  check belongs with MOD-16.
- `htui worker` refuses to start when it cannot create a tool host.
- MOD-75 (an MCP `ask_person` tool, leased by `hr/MOD-69`) would plug into this server.
- A `command_limits` editor and a Settings entry for the heavy-command list are not built; both are
  set in Postgres or fixed in the build.

# Plan: MOD-2 `agy` over ACP (milestone 6)

**Source PRD**: `.claude/prds/mod-2-agent-driver-chat.prd.md`
**Selected Milestone**: 6 (`agy` over ACP). Milestones 1–2 landed under
`.claude/plans/mod-2-driver-seam-registry.plan.md` (`5c717d0`..`34d5f04`), milestone 3 under
`.claude/plans/mod-2-live-acp-chat.plan.md` (`a142fbf`..`682a423`), milestone 4 under
`.claude/plans/mod-2-durable-history-replay.plan.md` (`81d247b`), milestone 5 under
`.claude/plans/mod-2-probe-autodiscovery.plan.md` (`fb626a8`). This plan continues their decision
(`D57`+) and task (`T29`+) numbering so a cross-reference never means two things. Milestones 7–9 are
out of scope; where one of them owns a seam this plan touches, the plan says where it stops.

**Design authority**: `docs/ANA-4.md` §4.5 (`agy` speaks ACP through Google's first-party
`agy_acp_server`, `transport: 'acp'`; the out-of-band `<GEMINI_HOME>/antigravity-acp/` configuration
and auth directory; the CLI contract deliberately not built), §4.6 (the glob tier, the per-platform
`args` append, "installed but unauthenticated is left disabled"), §5.3 (the `agy` seed row), §7
(`agy_acp_server` usage reporting unverified), §11 criteria 10 and 11, §11.14 (six of the eleven
open items are `agy`'s). `docs/ANA-2.md` §7 (MOD-4's skip predicate reads `probe->>'status'`).
`docs/REQUIREMENTS.md` `R-AGT-1`, `R-AGT-4`, `R-AGT-5`, `R-AGT-6`, `R-TUI-6`, `R-NF-3`.

**Requirements**: `R-AGT-4` (a second agent is a registry row, not a code path), `R-AGT-5` (a new
agent costs one row and at most one adapter — so nothing added here may be keyed on the string
`"agy"`), `R-AGT-1`/`R-TUI-6` (the same streamed chat, permissions and diffs as `claude`),
`R-AGT-6` (the probe records what this box can run), `R-NF-3` (no spawn on the UI task).

**Complexity**: Medium

**Routing**: PRD path, continued. Routed as **plan** on 2026-09-08 with the maintainer accepting
the verdict (only C3 fired — the `agy_acp_server` unknowns of ANA-4 §11.14 — and those are
empirical, answered by running the real adapter rather than by a document). Ultracode: not needed;
the work is a short serial chain gated on one live binary. Web research was requested at
route-accept and is folded into D57 and the Prerequisite section. Model assignment, maintainer's
call at CONFIRM: `code-architect` on **Fable 5.1**, implementers on **Opus 5**, the `rust-reviewer`
gate on **Fable 5.1**.

## Summary

Milestone 5 left `agy` provable only on paper. Its seed row (`crates/htui-core/seeds/agent_agy.json`)
resolves `agy_acp_server` through a glob whose Linux pattern is
`~/.local/share/htui/agents/antigravity-acp/*/agy_acp_server.par` with `args: ["--uid="]`, and the
probe knows how to append those args (`probe.rs:210-217`, `ResolvedTools::extra_args`) — but no such
file exists on this box, so criterion 10 was split (milestone 5 D56) and the glob, the append and
the `unauthenticated` mapping were proven by fixtures alone.

This milestone makes a second agent real, and in doing so closes the three defects milestone 5
recorded against itself:

- **H-3** — `tools::resolve` returns `ToolMap = BTreeMap<String, String>`, one string per tool, so a
  glob tool's per-platform `args` cannot survive it (`tools.rs:23-26`). The chat path would launch
  `agy_acp_server.par` **without** `--uid=`. The fix ANA-4 §4.6 and the milestone-5 blueprint both
  name: build the driver from `agent_box.probe.resolved`, which is by definition "what
  `AgentDriver::start` spawns" (`launch.rs:345-351`), instead of resolving a second time.
- **H-4** — `probe::status_for` maps *any* non-empty `authMethods` to `unauthenticated`
  (`probe.rs:919-925`). `claude`'s adapter reports none, so criterion 9 was unaffected; the
  Antigravity server reports four (`oauth-personal`, `oauth-business`, `gemini-api-key`,
  `agent-platform`) **whether or not** the box holds a credential, so an authenticated `agy` box
  would read `unauthenticated`, `enabled = false`, and MOD-4 would skip it.
- **H-2** — `open_session`'s timeout arm calls `task.abort()` (`acp/mod.rs:568-575`) while the child
  lives in the task's `Mutex<Option<Spawned>>` (`acp/mod.rs:811`), and `Spawned` has no `Drop` that
  kills. A handshake timeout therefore orphans the adapter — the exact failure `682a423` fixed for
  the actor-failure path and `fb626a8` fixed for the probe. `ChildGuard` (`launch.rs:595-657`)
  already exists and already signals a kill from `Drop`.

Everything else in the milestone is evidence: a live `agy_acp_server` handshake (criterion 10), a
live `agy` chat through the production runtime (criterion 11's second binary), and written answers
to the six `agy` items of ANA-4 §11.14.

## Prerequisite: the adapter must exist on this box (T29)

Verified 2026-09-08 on this box: `agy` **is** installed (`1.1.27`, mise shim at
`~/.local/share/mise/installs/aqua-google-antigravity-antigravity-cli/latest/agy`), and
`agy_acp_server` is **not** — `~/.local/share/htui/agents/` does not exist, and `~/.gemini/` holds
`antigravity-cli/` but no `antigravity-acp/`, which is the ACP server's own disjoint config
directory (ANA-4 §4.5). So the box is currently the *unauthenticated* case even once the server is
installed, which is convenient: criterion 10's stated half ("on an unauthenticated box the probe
records `unauthenticated` rather than `ready`") is provable before any credential exists.

The ACP registry (re-fetched 2026-09-08, `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`)
still lists `antigravity-acp` **1.1.1**, Google LLC, proprietary, five platform archives, no
checksums. This box's platform entry:

```
linux-x86_64  https://dl.google.com/agy-extensions/releases/linux/agy-acp-server-agy_acp_server_1.1.1-linux-x86_64.zip
              format: zip   cmd: ./agy_acp_server.par   args: ["--uid="]
```

Install (maintainer action, T29) is: download that zip, unzip the **whole** directory into
`~/.local/share/htui/agents/antigravity-acp/1.1.1/` (ANA-4 §4.6: keep the siblings — the Windows
bundle ships `localharness_external.exe` beside the server and may need it; whether the Linux
`.par` has an equivalent is a §11.14 answer this milestone produces), and `chmod +x
agy_acp_server.par`. The `+x` bit is load-bearing: `launch::spawn` resolves through
`which::which(&launch.command)` even for an absolute path (`launch.rs:682-690`), which rejects a
non-executable file.

Known field failure, checked and **not ours**: the `.par` aborts at startup on aarch64 kernels with
a 39-bit VA (`TCMalloc … MmapAligned() failed`, Google AI developer forum thread 180562). This box
is `x86_64`.

## Design decisions (settled here, not in code review)

| # | Decision | Why |
|---|---|---|
| D57 | **`htui` does not download the adapter.** The registry URL above is documented in `README.md` and in this plan; installation is a maintainer step, and the seed glob is the only contract between the two. | ANA-4 §4.6 records the registry as a *download coordinate*, not a mechanism, and treats the registry semver (`1.1.1`) as unrelated to the build tag the handshake reports (`agy_acp_server_20260818_01_RC01`). An installer inside MOD-2 would be a second unversioned update path for a proprietary binary, and `HTUI_TOOL_AGY_ACP_SERVER` already covers the box whose layout the glob does not understand (`tools.rs:33-50`). |
| D58 | **The driver prefers `agent_box.probe.resolved` and falls back to resolving.** `AcpAdapter::build` stops ignoring `on_box` (`acp/mod.rs:326-333`); when the snapshot is usable, `IoSource::Spawn` carries the recorded `ResolvedLaunch` and `io()` spawns it directly. "Usable" is: `probe.resolved` is `Some`, `probe.source == "probe"`, `status` is `ready` or `unauthenticated`, and the recorded `command` still exists on disk. Anything else resolves as today. | This is the H-3 fix and the only place the `--uid=` append exists (`probe.rs:210-217`). The disk check is not belt-and-braces: ANA-4 §4.6 records that `agy` self-updates in place, so a recorded path can be a version-numbered directory that is gone, and a chat must degrade to re-resolution rather than fail. `source == "probe"` because a `manual` row is a human's path, which `launch::resolve` already honours through the row itself. |
| D59 | **The credential check behind `unauthenticated` is declarative in the registry row, never keyed on an agent name.** `Discovery` gains an optional `credential { env: [..], files: [..] }`; a probe with a non-empty `authMethods` and **no** credential hit is `unauthenticated`, with a hit it is `ready`. A row that declares no `credential` block keeps milestone 5's rule verbatim (non-empty ⇒ `unauthenticated`). `agy`'s block declares two file candidates in the **existing** expander grammar (`probe.rs:490-536`: `%VAR%` anywhere, `~` leading only, an unset variable skips the pattern) — `%GEMINI_HOME%/antigravity-acp/acp_token.json` then `~/.gemini/antigravity-acp/acp_token.json`, first hit wins — plus `env: ["GEMINI_API_KEY"]` for the `gemini-api-key` auth method. | H-4. `R-AGT-5` forbids the alternative — a `match agent.name { "agy" => … }` is exactly "a second hard-coded agent", the degeneration the PRD's risk table names. ANA-4 §4.5 gives the file's location as an observed fact about the server's own layout, so it belongs in the row that describes that server. The `env` half is what makes the ToS-safer API-key path (D63) reachable without a second mechanism. |
| D60 | **Re-probe on spawn failure lands here; there is no in-row `cli` fallback, now or later.** A `DriverError::Spawn` at chat start marks the row for re-probe and surfaces the adapter's message. Maintainer decision at CONFIRM (2026-09-08): **a CLI agent is its own registry row**, a different agent from the ACP one — not a degraded second transport hiding inside the same row. Milestone 8 therefore builds the `cli` transport and the row that uses it; it does not build a runtime fallback between the two. | Milestone 5 D55 deferred "re-probe on spawn failure … with the `cli` fallback that gives it somewhere to fall back to", and the milestone-5 blueprint's section I listed both against milestone 6. The maintainer's framing settles it in the direction `R-AGT-4`/`R-AGT-5` already point: `agent.transport` is a property of a row, `DriverFactory` is keyed by transport (`registry.rs:96-107`), and a row that silently changed transport mid-session would make `DriverCaps` — which the chat tab and MOD-4 branch on — a value that can change under them. It also keeps this milestone's file set inside `htui-agent` + `agent_worker.rs`. |
| D61 | **H-2 is fixed by moving the session task's child into a `ChildGuard`**, not by teaching `open_session` to wait. `run_session`'s `Mutex<Option<Spawned>>` becomes a `Mutex<ChildGuard>`; `kill`/`stderr_tail` (`acp/mod.rs:1357-1382`) go through the guard's own methods. An aborted task drops the guard, whose `Drop` signals the kill. | The guard exists and is already the probe's answer to the same failure (`launch.rs:646-657`, `acp/handshake.rs:77-100`). Waiting for the task in the timeout arm would make a hung adapter a 120-second chat start instead of 60. |
| D62 | **The §11.14 `agy` answers are recorded as transcript fixtures and a table, and the mapper is amended only if the wire demands it.** No new `DriverEvent` variant: `§4.1`'s eleven are closed, and milestone 3's rule — unknown update kinds map through `other` — already covers a vendor shape. | Milestone 3 set this precedent for `claude` (recorded fixtures against adapter 0.48.0, two case *scripts* amended, no case added, no variant added). A second agent that needed a twelfth variant would be evidence the seam failed, and that is a finding worth surfacing rather than a change worth making quietly. |
| D63 | **Auth is the subscription OAuth flow, by maintainer decision at CONFIRM (2026-09-08)**, not the API key the plan first proposed. `agy_acp_server` runs its own login into `<GEMINI_HOME>/antigravity-acp/`; `htui` never sees the credential and cannot log in non-interactively (ANA-4 §4.5), so T34 runs after the maintainer has authenticated the server by hand. D59's credential probe therefore checks the token file first and the `GEMINI_API_KEY` variable second — both tiers stay, because the file is now the one that fires here. | The subscription is what the maintainer has and what the product targets (`billing: "subscription"` in the seed row). ANA-4 §10 risk 8 — third-party clients on a personal account flagged as a ToS breach, API key named as the mitigation — was put to the maintainer at CONFIRM with the 2026-09-08 web sweep confirming it is still repeated by every third-party adapter's README; the decision stands with the risk accepted and recorded here rather than silently. The registry entry being Google-authored and Google-hosted argues the ACP *protocol* path is sanctioned; it does not settle the account type. |
| D64 | **`agent.models` and `default_model` are filled from the live `session/new` answer, or stay empty.** If `configOptions` carries a model list, the seed gains it and `settings.acp.model_config_id` gains the option id; if it carries none, both stay as seeded and the fact is recorded. | ANA-4 §5.3 seeded `models: []` precisely because the list was unknown, and §4.4's model selection is by config-option **id** — a guessed id would select nothing and report success. |

## Patterns to Mirror

- `crates/htui-agent/tests/probe_live.rs` — the `#[ignore]`-by-default live suite: module doc saying
  what it spawns, why it burns no model tokens, and the exact `cargo test … -- --ignored` line. T33
  and T34 are its siblings, and T33 states its own precondition (the adapter is installed) rather
  than failing a build that never had it.
- `crates/htui-agent/src/acp/handshake.rs` — child owned by a `ChildGuard`, killed on every exit
  path. D61 makes `run_session` match it.
- `crates/htui-agent/src/probe.rs` `resolve_tool` / `ResolvedTools::extra_args` — one resolver, two
  callers (`tools.rs:5-7`). D58 keeps that shape: the driver gains a *reader* of the snapshot, not a
  third resolver.
- `crates/htui-core/seeds/agent_*.json` + `htui_core::model::agent::seed_rows` — a capability
  difference between two agents is a JSON difference, tested through `tests/extensibility.rs`.

## Files to Change

| File | Change |
|---|---|
| `crates/htui-agent/src/acp/mod.rs` | D61 (`Mutex<ChildGuard>` in `run_session`, `kill`/`stderr_tail` through it); D58 (`AcpAdapter::build` reads `on_box`; `AcpDriver::from_row_with_probe`; `IoSource::Spawn` carries an optional `ResolvedLaunch`; `io()` prefers it) |
| `crates/htui-agent/src/launch.rs` | D59: `Discovery` gains `credential: Option<CredentialProbe>` (`#[serde(default)]`) |
| `crates/htui-agent/src/probe.rs` | D59: credential resolution (env var set, or file exists after `${GEMINI_HOME}`/`~`/`%VAR%` expansion — the glob walker's expander, reused) feeding `status_for`; the snapshot records which tier answered |
| `crates/htui-core/seeds/agent_agy.json` | D59 `credential` block; D64 `models`/`default_model`/`model_config_id` if the live answer supplies them |
| `crates/htui/src/agent_worker.rs` | D60: a `DriverError::Spawn` at chat start schedules the same `run_reprobe` the staleness path uses (`agent_worker.rs:528-545`) and reports the adapter's message |
| `crates/htui-agent/tests/agy_live.rs` | **new** — criterion 10 live (T33) |
| `crates/htui/tests/chat_live_agy.rs` | **new** — criterion 11's second binary and the §11.14 answers (T34) |
| `crates/htui-agent/tests/{probe.rs,launch.rs,acp_map.rs}` | credential-tier cases, `probe.resolved`-backed build cases, any mapper amendment D62 admits |
| `crates/htui-agent/tests/fixtures/` | recorded `agy` transcript(s) |
| `README.md`, `HANDOFF.md`, PRD milestone row | close-out (T36) |

## Tasks

TDD per repo convention: the test that fails for the stated reason comes first.

### T29: Install the adapter and record what it is (blocks T33/T34 only)

Download the D57 URL, unzip into `~/.local/share/htui/agents/antigravity-acp/1.1.1/`, `chmod +x`.
Record: the `.par`'s size, what sits beside it, `file` output, and whether it starts at all. Nothing
in the code depends on this task; T30–T32 and T35 proceed without it. The download itself was
authorised by the maintainer at CONFIRM (2026-09-08) and runs in this session; the **OAuth login**
(D63) is the maintainer's own interactive step against `agy_acp_server`, and T34 waits on it.

### T30: H-2 — a handshake timeout kills the adapter (independent; `acp/mod.rs`)

Test first: `open_session` against a transport that accepts the connection and never answers
`initialize`, with `HANDSHAKE_TIMEOUT` shortened, asserts the child is gone. Then D61's guard swap.

### T31: H-3 — the driver spawns `probe.resolved` (serial after T30 — same file)

Tests first: (a) a row whose `on_box.probe.resolved.args` contain a marker argument spawns *with*
it; (b) a snapshot whose recorded command no longer exists falls back to resolution; (c) a
`source: manual` snapshot is not used as a launch. Then D58.

### T32: H-4 — the declarative credential check (independent of T30/T31; `launch.rs`, `probe.rs`, seed)

Tests first, over a temp `GEMINI_HOME`: non-empty `authMethods` + no credential ⇒
`unauthenticated`; + a credential file ⇒ `ready`; + `GEMINI_API_KEY` set ⇒ `ready`; a row with no
`credential` block keeps the milestone-5 rule; `claude`'s empty `authMethods` is `ready` either way.
Then D59.

### T33: Criterion 10 live (serial after T29 and T32)

`crates/htui-agent/tests/agy_live.rs`, `#[ignore]` by default: the **unmodified** seed `agy` row
resolves `agy_acp_server` through the glob, appends `--uid=`, completes `initialize` with
`protocolVersion == 1`, and — with no credential present — records `unauthenticated`, `enabled =
false`. Then the process check: no surviving `agy_acp_server` after the probe, mirroring
`probe_live.rs`'s survivor assertion. Answers the §11.14 `.par` mechanics item: what `--uid=` does
(compare a run without it), and whether anything must sit beside the server.

### T34: A live `agy` chat through the production runtime (serial after T31 and T33)

`crates/htui/tests/chat_live_agy.rs`, `#[ignore]`, modelled on `crates/htui/tests/chat_live.rs`:
one turn that streams into the store, with a credential per D63. It answers the remaining five
§11.14 items — `usage_update` presence and field, `session/request_permission` in `default` mode
with its option ids and kinds, the edit-proposal shape (standard `tool_call` + `diff`, or vendor),
the `session/new` model list and `configOptions` (D64), and the capability banner the row produces.
Transcript recorded as a fixture; mapper amended only under D62.

### T35: Re-probe on spawn failure (independent; `agent_worker.rs`)

Test first: a chat whose driver fails to spawn reports the adapter's message and schedules a
re-probe, and does so **off** the worker's `select!` arm (`R-NF-3`). Then D60.

### T36: Close-out (serial, last)

`README.md` gains the adapter install steps and the `HTUI_TOOL_AGY_ACP_SERVER` escape hatch; the
§11.14 answers land as a table in this plan and in the eventual `docs/decisions/mod/mod-2.md`;
`HANDOFF.md` gains the phase note; the PRD's milestone 6 row goes `complete`.

## Validation

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres cargo test --workspace --all-features

# T33, explicitly (spawns the real adapter, burns no model tokens)
cargo test -p htui-agent --features test-support --test agy_live -- --ignored --nocapture

# T34, explicitly (burns model tokens against the maintainer's credential)
cargo test -p htui --features demo,test-support --test chat_live_agy -- --ignored --nocapture
```

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The adapter cannot be installed or authenticated on this box | Medium | Medium | T30–T32 and T35 are the milestone's code and none of them needs it; criterion 10 stays split exactly as milestone 5 left it, and the phase note says so rather than claiming a green it did not earn (TOOL-2's lesson) |
| Driving Antigravity from `htui` breaches Google's ToS on a personal account | Medium | High | **Accepted by the maintainer at CONFIRM** (D63: subscription OAuth, not the API key): the risk was stated with its sources and the decision was taken knowingly. ANA-4 §10 risk 8 carries it, the Settings agent section states account modes plainly, and `htui` never handles the credential — `agy_acp_server` owns its own login |
| `probe.resolved` becomes a stale-path trap after an `agy` self-update | Medium | Medium | D58's disk check + fallback to resolution; milestone 5's 24 h `PROBE_TTL` re-probe refreshes the row |
| The `agy` wire needs a twelfth `DriverEvent` variant | Low | High | D62 treats that as a finding to surface, not a change to make quietly; `other` carries an unknown kind meanwhile |
| A credential check that reads a token file leaks it into a log | Low | High | The probe records *which tier answered*, never the value; `RedactedEnv` (`driver.rs:298-308`) is the existing precedent |

## Verified claims

Checked against the tree at `fb626a8` on 2026-09-08, before CONFIRM.

| Claim | Verdict | Evidence |
|---|---|---|
| `AcpAdapter::build` ignores `on_box` | true | `acp/mod.rs:326-333` (`_on_box`) |
| `open_session`'s timeout arm only aborts the task | true | `acp/mod.rs:568-575` |
| The session task's child is a `Mutex<Option<Spawned>>` with no killing `Drop` | true | `acp/mod.rs:811`; no `impl Drop for Spawned` in `launch.rs` (`launch.rs:443-445`) |
| `ChildGuard` exists and signals a kill from `Drop` | true | `launch.rs:595-657` |
| `ResolvedLaunch` is `Serialize + Deserialize`, so a snapshot round-trips | true | `launch.rs:350-351` |
| `ProbeSnapshot` carries `resolved: Option<ResolvedLaunch>` and `source` | true | `probe.rs:870-888` |
| The platform `args` append exists only on the probe path | true | `probe.rs:210-217`; `tools.rs:23-26` states the limit |
| `status_for` is "non-empty `authMethods` ⇒ `unauthenticated`", with no credential input | true | `probe.rs:919-925` |
| The chat path re-resolves tools at `start` | true | `acp/mod.rs:268-279` |
| `launch::spawn` runs `which` even on an absolute command | true | `launch.rs:682-690` |
| `PROBE_TTL` is 24 h and re-probe is scheduled off the worker arm | true | `agent_worker.rs:61`, `:528-545` |
| The expander handles `%VAR%` and a leading `~`, and skips a pattern whose variable is unset — so a `${GEMINI_HOME}` spelling would have needed new grammar | true | `probe.rs:490-536`; D59 uses two `%VAR%`/`~` candidates instead |
| `RedactedEnv` lives in `driver.rs`, not `launch.rs` | true | `driver.rs:298-308`; the risk table's citation was corrected to it |
| `crates/htui/tests/chat_live.rs` exists as T34's model | true | directory listing of `crates/htui/tests/` |
| The seed `agy` row is `transport: acp`, glob + `["--uid="]` on Linux, `models: []` | true | `crates/htui-core/seeds/agent_agy.json:1-45` |
| `agy_acp_server` is absent from this box; `agy` 1.1.27 is present | true | `~/.local/share/htui` does not exist; `agy --version` → `1.1.27` |
| `~/.gemini/antigravity-acp/` does not exist (so the box is the unauthenticated case) | true | `ls ~/.gemini` → `antigravity`, `antigravity-cli`, `antigravity-ide`, `commands`, `config`, `extensions` |
| The ACP registry still lists `antigravity-acp` 1.1.1 with the linux-x86_64 zip and `["--uid="]` | true | registry.json fetched 2026-09-08 |
| The registry entry carries no checksum for any Antigravity platform | true | same fetch |
| T30/T31 touch `acp/mod.rs`; T32 touches `launch.rs`/`probe.rs`/seed; T35 touches `agent_worker.rs` | file sets intersect only for T30/T31 | stated per task above; T30→T31 serial, T32 and T35 independent |

## Acceptance

1. A handshake timeout leaves no adapter process behind (H-2 closed).
2. A row with a usable `probe.resolved` spawns exactly what the probe recorded, `--uid=` included;
   a stale recorded path degrades to resolution rather than failing the chat (H-3 closed).
3. `unauthenticated` means "authMethods and no credential", declared in the row; `claude` is
   unaffected (H-4 closed).
4. ANA-4 §11 criterion 10 is proven live on this box, or explicitly still split with the reason.
5. A live `agy` chat streams through the same code paths as `claude`, differing only by registry row
   and capability banner; no `DriverEvent` variant was added.
6. The six `agy` items of ANA-4 §11.14 are answered in writing.
7. Full suite green on Linux with Postgres live; Windows clippy target green.
8. `rust-reviewer` gate clear (`.claude/workflow-config.json`).

## Close-out

Phase 6 note appended to `HANDOFF.md`'s MOD-2 entry per `.claude/rules/workflow-docs.md` lifecycle
step 4 (MOD-2 stays one open item until milestone 9), PRD milestone 6 row → `complete`, validator
green, commits reported. Push only when agreed.

### Status, 2026-09-09 (`acf16f7`)

T29–T33, T35 and T36 are done; **T34 alone is outstanding**, and it is blocked on a maintainer
action rather than on code: `agy_acp_server` must be authenticated through the vendor's own flow
(D63) before a live chat can open a session. The PRD row is therefore `in-progress`, not `complete`.

| Task | Outcome |
|---|---|
| T29 | `agy_acp_server` 1.1.1 installed under `~/.local/share/htui/agents/antigravity-acp/1.1.1/`; 1.88 GB ELF launcher wrapping CPython, `localharness_external` beside it |
| T30 | D61 landed; a handshake timeout kills its child. The blueprint's "no polling needed" was wrong — `Drop` only *signals*, and the group is still visible for ~2 ms — so the assertion polls a bounded window |
| T31 | D58 landed. Blueprint B.2's `launch_for` body did not compile (irrefutable let-else without `test-support`); a `cfg`-gated `match` replaced it |
| T32 | D59 landed. `Discovery`'s new field forced a one-line `credential: None` in `tools.rs`'s own `#[cfg(test)]` literal, which no file table anticipated |
| T33 | **Criterion 10 proven live.** `--uid=` is mandatory on Linux; `localharness_external` is not needed for the handshake; the model list is not learnable while unauthenticated (D64 falls back) |
| T35 | D60 landed, with the double-probe guard promoted from per-chat to runtime-wide at the review gate |
| T36 | README install section, HANDOFF phase note, PRD row, this section. The two ANA-4 corrections are recorded in `HANDOFF.md`, not applied to the ANA (maintainer-only) |

### Review gate

`rust-reviewer` (per `.claude/workflow-config.json`) returned **no CRITICAL and no HIGH**; its three
MEDIUM and five LOW are all applied in `acf16f7`. Three are worth carrying forward:

- **The reasoning behind D58's `unauthenticated` arm was wrong, though the arm is right.** H-4 justified
  it as "the vendor's auth error is a better message than a second resolution". Neither half held: the
  message was being swallowed (fixed here), and on Linux a second resolution produces not a worse
  message but a dead adapter, because the recording is the only launch carrying `--uid=`. The doc and
  blueprint H-4 now say that.
- **D58 needed a fifth staleness input** (MEDIUM-2): `needs_reprobe` was age-only, so a hand-edited
  `agent.launch` would be ignored for up to 24 h while chats kept spawning the old recording.
  `agent.updated_at > probed_at` now counts as stale, and a snapshot whose `transport` no longer
  matches the row is refused.
- **Blueprint H-1's zombie is smaller than stated** and its speculated reaper task is **withdrawn**:
  tokio's `Reaper::drop` queues an unwaited child for the next `SIGCHLD`, and Windows has no zombie.

### What T34 must still do

Re-run `agy_live.rs` case 3 for the model list (D64), then the live chat itself, answering ANA-4
§11.14's remaining three `agy` items: `usage_update` presence and field, `session/request_permission`
in `default` mode with its option ids and kinds, and the edit-proposal shape. The blueprint's G-T34
holds unchanged.

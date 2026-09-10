# Plan: MOD-2 degraded CLI transport (milestone 8)

**Source PRD**: `.claude/prds/mod-2-agent-driver-chat.prd.md`
**Selected Milestone**: 8 (Degraded CLI transport). Milestones 1–2 landed under
`.claude/plans/mod-2-driver-seam-registry.plan.md` (`5c717d0`..`34d5f04`), milestone 3 under
`.claude/plans/mod-2-live-acp-chat.plan.md` (`a142fbf`..`682a423`), milestone 4 under
`.claude/plans/mod-2-durable-history-replay.plan.md` (`81d247b`), milestone 5 under
`.claude/plans/mod-2-probe-autodiscovery.plan.md` (`fb626a8`), milestone 6 under
`.claude/plans/mod-2-agy-acp.plan.md` (`acf16f7`) with its `T34` closed inside milestone 7, and
milestone 7 under `.claude/plans/mod-2-quota-caps.plan.md` (`142beb1`..`acec1b8`). This plan
continues their decision (`D79`+) and task (`T49`+) numbering. Milestone 9 (prompt assembler and
preview) is out of scope; where it owns a seam this plan touches, the plan says where it stops.

**Design authority**: `docs/ANA-4.md` §4.3 (what the CLI fallback can and cannot do; the
`DriverCaps` triple it must report; the `--permission-prompt-tool` route deferred to MOD-11), §4.4
(the verified flag set and the invocation line; the `--input-format stream-json` follow-up
mechanism; `--bare`; the `tool_use_id` join key; the unverified cancellation semantics), §6.2 (the
`claude -p --output-format stream-json` → `EventKind` mapping table, row by row), §7 (what the
`claude` CLI can report: the inline `rate_limit_event` blob, `result.total_cost_usd`,
`modelUsage[*]`, and the `usage_scope: "model_usage"` convention), §8 (`src/cli/mod.rs` and
`src/cli/claude.rs` as the module layout, and the fixture-plus-`insta` regression net), §11 criteria
1–8 and 11, §11.14 (the two CLI items this milestone closes and the two it does not).
`.claude/plans/mod-2-agy-acp.plan.md` **D60** (maintainer decision, 2026-09-08): a CLI agent is its
own registry row, not a degraded second transport inside the ACP row — milestone 8 builds the row.
`docs/REQUIREMENTS.md` `R-AGT-3`, `R-AGT-4`, `R-AGT-5`, `R-HIS-1`, `R-SEC-3`, `R-NF-3`, `R-TUI-6`,
`R-TUI-8`.

**Requirements**: `R-AGT-3` (**must** — a CLI stream adapter for agents without ACP, parsing the
headless streaming output into the same event kinds), `R-AGT-4` (the row that names it), `R-AGT-5`
(nothing added here is keyed on an agent name; the dialect is declared in the row as
`settings.cli.stream` and the factory keys on `cli/<stream>`), `R-HIS-1` (its events persist like
any other transport's), `R-SEC-3` (its rows are scrubbed like any other transport's), `R-NF-3` (no
blocking work on the UI task), `R-TUI-6` (the chat tab renders it, banner included), `R-TUI-8` (the
Settings section shows the row).

**Complexity**: Large

**Routing**: PRD path, continued. Routed as **plan** on 2026-09-10; one criterion fired (C3 — the
two empirical `claude` CLI unknowns of §11.14, answered by running the real binary rather than by a
document), below the ≥2 PRD threshold, and the PRD already exists. Ultracode: not needed — the
chain is a single crate's module family plus two live probes, and its two halves are gated on one
binary.

**Maintainer constraints taken at routing (2026-09-10)**: MOD-11 is **not** a prerequisite (see
"Why MOD-11 is not a blocker" below), and the work runs **on `main` with no branch and no
worktree**; if a branch turns out to be unavoidable it is merged back to `main` before close-out.

**CONFIRMED 2026-09-10.** The maintainer accepted **D79** (two rows; the ANA-4 §5.3 / §4.4
amendment is theirs to make and is recorded here per the milestone-5/6/7 precedent), **D80** (the
capability arm, not a skip), **D88** (the name-keyed seed top-up), and **D90** (`SessionSpec`
gains `budget_micros`). **D89 was overruled**: the row keeps the name `claude-cli` and the Settings
`name` column is widened instead — see D89 for the arithmetic that forced `Min(10)` rather than the
literal `Length(128)`, and for the donor that pays for it. **D81** was never a choice to make now:
it is a live finding T55 produces before the code is written. Work runs **on `main`, no branch and
no worktree**, at the maintainer's instruction.

---

## Why MOD-11 is not a blocker

The question was asked at routing and the tree answers it: milestone 8 is *defined* as the degraded
path, and the degradation is declared rather than hidden.

- `docs/ANA-4.md` §6.2 marks `permission_request`, `edit_proposal` and `plan` **"not produced by
  this transport"**; `permission_request` is available "only out of band via
  `--permission-prompt-tool` pointed at `htui`'s MCP server (MOD-11)" (`ANA-4.md:1043-1046`).
- §4.3 states the contract this milestone must satisfy verbatim: "the CLI transport reports
  `DriverCaps { permission_requests: false, edit_proposals: false, plans: false }`, runs with the
  `--permission-mode` configured in `agent.settings.cli`, and synthesizes `permission_answer` rows
  with `by: "policy"`" (`ANA-4.md:550-553`).
- The PRD puts MOD-11 **out of scope** for MOD-2 and says the gap "stays declared until MOD-11
  lands" (`prd:120-121`), while keeping the CLI transport itself **in** scope with `R-AGT-3` a
  `must` (`prd:87-89`).
- The code already carries the contract: `registry.rs:159-171` returns exactly that `DriverCaps`
  triple for `Transport::Cli`, and `chat/mod.rs:323-333` already renders the banner from `caps`,
  not from `agent.transport`.

So MOD-11 changes what a *future* CLI session can do; it changes nothing this milestone must
build. The one place it bites is the conformance suite, which is D80.

---

## Summary

`htui` can hold a live conversation with an agent that speaks ACP, and with nothing else. Milestone
8 makes an agent that speaks only its own headless JSON stream reach the same chat tab, the same
recorder, the same store rows and the same replay — losing exactly three event kinds, and saying so
on screen.

Three things are already true, which is why this milestone is smaller than its position in the list
suggests:

- **The seam is built and documented for this.** `DriverFactory` keys on `cli/<settings.cli.stream>`
  (`registry.rs:122-130`), `caps_from` already returns the §4.3 triple for `Transport::Cli`
  (`registry.rs:159-171`), `CliSettings { stream, permission_mode, extra_args }` already
  deserialises (`launch.rs:358-374`), `QuotaSource::CliRateLimitEvent` and `CliStatusLine` already
  exist in the enum, `UsageScope::ModelUsage` already names §7's summing convention, and
  `probe.rs:1444-1448` already declines tier 2 for a non-`acp` row. `registry.rs:62-63` names this
  milestone's change as "one line".
- **The chat tab needs nothing.** `caps_banner` is computed from `DriverCaps` and already emits a
  line per missing capability (`chat/mod.rs:320-333`).
- **The flags are real.** Every flag §4.4 lists was re-checked against the `claude` on this box
  (2.1.267) on 2026-09-10: `--bare`, `--max-budget-usd`, `--input-format`, `--output-format`,
  `--include-partial-messages`, `--permission-mode`, `--session-id`, `--resume`, `--add-dir`,
  `--forward-subagent-text`, `--verbose` all present.

What does not exist yet: `src/cli/mod.rs`, `src/cli/claude.rs`, the registry row that selects them,
a third conformance binding, and live answers to §11.14's two CLI questions.

---

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Module layout | `crates/htui-agent/src/acp/{mod,map}.rs` | supervisor/session task in `mod.rs`, wire→`DriverEvent` mapping alone in `map.rs`; `map.rs` imports no process type and is unit-testable from a fixture line |
| Driver construction | `crates/htui-agent/src/acp/mod.rs:548-559` (`AcpAdapter::build` → `AcpDriver::from_row_with_probe`) | a unit struct implementing `TransportBuilder` that defers to a `from_row_with_probe` constructor reading `agent.launch` + `on_box.probe.resolved` (D58) and never re-resolving |
| Child ownership | `crates/htui-agent/src/launch.rs:734` (`ChildGuard`), `probe.rs:982` (`run_bounded`) | every spawn owned by a guard that kills the tree on **every** exit path — timeout, actor failure, garbage, success, dropped future |
| Session task | `crates/htui-agent/src/acp/mod.rs` | one task per session owning the whole child future; the `AgentSession` handle owns only channel endpoints (`driver.rs:388-389`) |
| Errors | `crates/htui-agent/src/error.rs` | `DriverError::{Spawn, Transport, Closed, Unresolved, Unsupported, UnknownAdapter}`; a named variant per cause, never a stringly error |
| Unknown wire shapes | `acp/map.rs` + `ANA-4.md:1028-1031` | decode against a non-exhaustive shape with a wildcard arm; an unknown kind lands in `other` **unchanged** rather than breaking the build |
| Fixtures | `crates/htui-agent/tests/fixtures/*.jsonl` + `tests/acp_map.rs` | a recorded transcript replayed through the mapper with an `insta` snapshot of the resulting `Vec<SessionEvent>` |
| Conformance binding | `crates/htui-agent/tests/acp_conformance.rs` | a `CaseHarness` that turns a `Script` into this transport's wire traffic, importing no SDK type on the agent side; the row comes from `seed_rows`, never hand-built |
| Live test | `crates/htui-agent/tests/agy_live.rs`, `crates/htui/tests/chat_live_agy.rs` | `#[ignore]`d, spawns the real binary, asserts no surviving children, records a transcript fixture the offline tests then replay |
| Seeds | `crates/htui-core/seeds/agent_*.json` + `model/agent.rs:130-154` | one compiled-in JSON document per row, `AgentSeed` deserialised into `Agent`; `seed_rows_match_ana4_5_3` asserts the file against the ANA |

---

## Decisions

| # | Decision | Why |
|---|---|---|
| **D79** | **A second registry row, `claude-cli`, and the `cli` block leaves the `claude` row.** New seed `crates/htui-core/seeds/agent_claude_cli.json` with `name: "claude-cli"`, `transport: "cli"`, `settings.cli { stream: "claude_stream_json", permission_mode: "acceptEdits", extra_args: ["--bare"] }`, `launch.command: "${claude}"` and the `claude` discovery tool alone (no `node`, no `npx`, no `claude_agent_acp`), `discovery.handshake: false`, `quota.source: "cli_rate_limit_event"`, `usage.scope: "model_usage"`, `billing: "subscription"`. The `settings.cli` block is **removed** from `agent_claude.json`. **This amends `docs/ANA-4.md` §5.3 and §4.4's "Registry consequence" paragraph** (`ANA-4.md:658-662`), which said `claude` is one row with the CLI recorded as an in-row degradation. | D60 already settled the direction at milestone 6's CONFIRM and this is where it is spent. Two rows is also what the shipped code wants: `adapter_id_from` and `caps_from` both branch on `agent.transport` (`registry.rs:122-172`), so a single row cannot report two capability profiles, and `DriverCaps` is a value the chat tab and MOD-4 branch on — it must not change under them mid-session. Leaving a dead `settings.cli` block on the `acp` row would leave two sources of truth for one dialect. Recorded here rather than in the ANA, per the milestone-5/6/7 precedent (an ANA edit is maintainer-only). |
| **D80** | **A capability-gated case, not a skipped one: `run_case` reads `driver.caps()` and asserts the *other* side of the contract.** The three cases that exercise capabilities a CLI transport does not have — `cancel_answers_parked_permissions`, `edit_proposal_deduped_per_call_and_path`, and the permission half of `rejected_tool_gets_failed_result` — gain a second arm. With the capability: today's assertions, unchanged. Without it: the case asserts that the transport emits **no** such row, that the recorder still closes every open tool call with a synthesized `failed` result, and that a cancel still ends the turn with `done{stop_reason:"cancelled"}`. `CASES` does not grow and no name changes. | Criterion 1 is "adding a transport adds no case" (`ANA-4.md:1340-1341`), not "every transport can do everything". A skip would satisfy the letter and drop the coverage; an arm keeps the case a statement about *both* transports and turns `DriverCaps` from a banner string into something the suite actually enforces. The alternative — a CLI harness that fabricates permission requests the real stream can never carry — would prove the harness, not the transport, which is the mistake `acp_conformance.rs:5-10` was written to avoid. |
| **D81** | **Cancel is stdin-close → SIGINT → grace → kill the group/job.** `AgentSession::cancel` closes the child's stdin (the documented end-of-input for `--input-format stream-json`), sends SIGINT on unix, waits the existing grace window, then kills the process group (unix) / job object (Windows), where Windows has no SIGINT and goes straight to the kill after the stdin close. **T55 verifies the middle step live before the code is written**, and the plan's `done` mapping follows what it finds. | §4.4 leaves this "**Unverified — MOD-2 must confirm**" with an explicit hypothesis (SIGINT ends the turn cleanly, SIGTERM leaves it unfinished at exit 143) and says "the driver's `cancel` depends on it" (`ANA-4.md:654-656`). Guessing here would put an unverified claim in the one path criterion 5 measures. |
| **D82** | **Thinking blocks are probed, then mapped.** T56 runs a thinking-enabled turn with `--include-partial-messages` and records the fixture; `cli/claude.rs` maps `assistant.content[].type == "thinking"` to `thought` and its `stream_event` deltas to `thought` chunks, per §6.2's row. If the live shape disagrees with §6.2, the fixture wins and the plan records the amendment. | §11.14's fourth item, unverifiable from a document: "thinking was off in the captured runs" when ANA-4 was written (`ANA-4.md:1040`). |
| **D83** | **`--max-budget-usd` carries `project.settings.per_token_cap_run` when it is set, as a *second* cap, and the recorder's own cap is unchanged.** Micros → USD with the conversion documented at the call site; an absent cap passes no flag. A breach detected server-side arrives as a `result` with an error subtype and maps to `error` + `done` like any other; the recorder's `enforce_breach` (D69) still fires first when the client-side estimate crosses first. | §7: "For `claude` the driver additionally passes `--max-budget-usd` (CLI) so a second, server-side cap exists; both figures are documented client-side estimates" (`ANA-4.md:1148-1150`), and milestone 7's plan explicitly deferred it here (`mod-2-quota-caps.plan.md:403`). |
| **D84** | **`system/init` is the `session_started` banner, and the session id is `htui`'s own.** The driver mints the UUID, passes `--session-id <uuid>`, and writes the banner as the step's first `other` row with the same key set milestone 3 established (`session_id`, `protocol_version`, `agent_name`, `agent_version`, `models`), sourcing `protocol_version: null`, `agent_version` from `system/init`, and `models` from the configured row. `AgentSession::session_ref` returns that id, which is what a later `--resume` uses. | The conformance case `session_banner_is_first_other_row` is in `CASES` and adding a case is forbidden, so the CLI transport owes a banner. §4.4's "session banner as the step's first `other` row" is transport-neutral by construction, and `--session-id` is what makes the id ours to mint rather than the agent's to report. |
| **D85** | **`permission_answer` rows are synthesized with `by: "policy"` and `denied: true`, and nothing else claims to be a permission.** Sources: `result.permission_denials[]` and `system/permission_denied`. No `permission_request` row is ever written by this transport. | §4.3 and §6.2 both state it (`ANA-4.md:552-553`, `:1045`); the row is what makes a denial visible in the transcript at all, and marking it `policy` is what keeps it distinguishable from a user's answer. |
| **D86** | **Usage sums `modelUsage[*]`, not `result.usage`, and records `usage_scope: "model_usage"`.** `inputTokens`, `outputTokens`, `cacheReadInputTokens`, `cacheCreationInputTokens` summed across models; `cost_micros` as the **delta** since the previous `usage` row of the step, `cost_micros_total` cumulative from `result.total_cost_usd`. `rate_limit_event` drives the quota latch through the existing `normalize(QuotaSource::CliRateLimitEvent, …)`. | §7: `result.usage` "counts only the top-level loop and undercounts as soon as a subagent runs" (`ANA-4.md:1099-1103`). The delta convention is what makes `run_step.usage` a plain sum, which is criterion 7 — already proven for ACP in milestone 7 and inherited unchanged here. |
| **D87** | **`DriverFactory::with_acp` becomes `DriverFactory::production`.** Eight call sites, mechanical. | The name becomes false the moment `cli/claude_stream_json` is registered beside `acp`, and a factory constructor that lies about its contents is exactly the trap `registry.rs:80-85`'s `adapter_ids` exists to prevent. |
| **D89** | **Maintainer decision at CONFIRM (2026-09-10): the row keeps the name `claude-cli`; the `name` column gets the width instead, and becomes the table's flexible column.** `name` takes `Constraint::Min(10)`, `on this box` takes `Constraint::Length(13)`. **This reverses D76's ranking and T48's `12 → 8` narrowing** — `name` is now first, not third-donor — and the comment block at `agents.rs:1021-1066` is rewritten to record the new ranking rather than left describing one that no longer holds. **The stated price, which the maintainer can overrule by naming a different donor**: `on this box` fixes at 13, so `unauthenticated` (15) renders `unauthenticat` at every width. | The instruction was "settings name should be wider like 128"; a fixed `Length(128)` is arithmetically impossible and `Max(128)` would silently deliver nothing. The bordered section draws **98** columns at the 100-column snapshot width (`agents.rs:1021-1022`, `tests/settings.rs:49`); with `column_spacing` 1 across 7 gaps the eight columns share exactly **91**, and today's widths sum to exactly 91 (8+9+12+6+21+7+13+15). Every one of the other seven already sits at its own floor — `transport`/`models`/`enabled` at their headers, `billing` at `subscription` (12), `default` at `gemini-3.7-flash-high` (21, D76's #1), `quota` at `100% to 09-08` (13) — so **any `name` wider than 8 at 98 columns must take those characters from a floor**; there is no arrangement where it is free. `Min(10)` is what makes the intent real rather than nominal: 10 is `claude-cli` whole at the narrow guard width, and every character a wider terminal adds goes to `name` first — 30 at a 120-column terminal, 70 at 160, which is the "like 128" behaviour without the impossible constant. The donor is `on this box` because D76 itself ranked its slack last (`agents.rs:1023-1024`) and its cells are status words, where `default`'s is a selection coordinate and `quota`'s is an exhaustion warning. |
| **D90** | **`SessionSpec` gains `budget_micros: Option<i64>`, and the CLI adapter is the only transport that reads it.** The chat path already computes the per-run cap in micros before the turn (`agent_worker.rs:2307-2312`, `RunCap { micros, grace }`); D83's `--max-budget-usd` needs the same figure on the argv, which is decided at `start`, not at `build`. ACP ignores the field — the protocol has no such knob. | `TransportBuilder::build` takes `(agent, on_box, caps)` and no run context (`registry.rs:38-43`), so the cap cannot arrive that way; `SessionSpec` is the per-session context and already carries `cwd`, `extra_dirs`, `env`, `model`, `retain_raw` and `resume` (`driver.rs:252-273`). Adding a field is additive for the fake and the ACP transport and keeps the recorder's cap and the server-side cap reading **one** number, which is what makes them comparable when they disagree. |
| **D88** | **Existing boxes get the new row by a name-keyed top-up, not by a migration.** `PgStore` gains `seed_missing_agents`, called where `seed_if_empty_as` is called, inserting any `seed_rows` name absent from the table with `ON CONFLICT (name) DO NOTHING`. **Alternative offered at CONFIRM**: do nothing and document that an already-seeded box adds the row by hand until MOD-23 ships the editor. | `seed_if_empty_as` fires only when the `agent` table is **empty** (`model/agent.rs:107`), so every box that has ever launched — including this one — would never see `claude-cli`, and the milestone would be verifiable only on a fresh database. The cost of the top-up is that a row the maintainer *deleted* comes back; MOD-23's model is `enabled = false`, not deletion, so the exposure is small. A MOD-2 `0003` migration is refused outright: `0003_orchestration.sql` is MOD-4's and the collision would be worse than the problem. |

---

## Decisions taken after the blueprint (2026-09-10)

The `code-architect` blueprint (`.claude/plans/mod-2-cli-transport.blueprint.md`) returned nine
errors in the plan above, three of them blocking. Each was re-verified against the tree before being
put to the maintainer. These three are the maintainer's answers; the six corrections are folded into
the tasks without a decision line.

| # | Decision | Why |
|---|---|---|
| **D92** | **`--bare` is dropped from the seed's `extra_args`, and `docs/ANA-4.md` §4.4 is amended to say why.** The supervisor therefore buffers events until `system/init` arrives, because without `--bare` hook events precede it and D84's banner must still be the step's first `other` row. | `claude --help` on 2.1.267: `--bare` skips "keychain reads" and "Anthropic auth is **strictly** `ANTHROPIC_API_KEY` or `apiKeyHelper` (OAuth and keychain are never read)". Subscription billing *is* the OAuth login, so D79's `billing: "subscription"` row plus `--bare` is a row that contradicts itself — every live task would fail auth before reading a byte of stdin. §4.4 recommended `--bare` as "the documented recommendation for scripted callers" without knowing this. Maintainer confirmed the drop at the blueprint gate. |
| **D93** | **`permission_answer` gets a real twelfth `DriverEvent` variant, not an `other` row the recorder rewrites.** `DriverEvent::PermissionAnswer` carries the existing `AnsweredBy::Policy`, and the recorder stamps that one variant `role: htui` rather than the `agent` every other driver row gets. **This amends `docs/ANA-4.md` §4.1**: `permission_answer` moves from "htui-authored only" to "authored by `htui` *or* reported by a transport that answered by policy", and `driver_contract.rs`'s identity becomes `14 − 2 = 12`. **No migration**: `EventKind::PermissionAnswer` and the `chk_event_kind` CHECK already carry the kind (`0001_init.sql:524-527`). In the same spirit, `rate_limit_event` maps to `EventKind::Usage` — a row with null token fields carrying the quota blob — rather than to `other`. | Maintainer instruction at the blueprint gate: add the event, do not smuggle it through `other`. The blueprint's `Other`-conversion route was proposed only to avoid a twelfth variant, and the maintainer overruled that trade — a typed event is what makes the denial visible to the chat tab and to replay without a string comparison on an `update` field. The boundary that survives is the **schema**: any kind *outside* the fourteen in `chk_event_kind` would need a migration, and `0003` is MOD-4's, so a recognizable shape gets a typed variant **only where an existing `EventKind` is its destination**. `system/init` stays `other` (D84's banner, pinned by `session_banner_is_first_other_row`); hook, plugin and unrecognized `stream_event` shapes stay `other` by §6.2's design, which is the extension point, not a dumping ground. |

| **D91** | **`DriverCaps` gains `usage_mid_turn: bool`, and D80's arms extend to six cases.** `true` for ACP, `false` for the CLI dialect, whose cost arrives once on the terminal `result`. `usage_deltas_sum_to_step_usage`, `quota_blob_latches_agent_box` and `run_cap_breach_cancels_within_one_event` gain a turn-end-form arm beside the three D80 already named. `CASES` stays at 15. | The three cases each assume cost is reported *during* a turn. Over the `claude` dialect it is reported once, at the end (`docs/ANA-4.md` §6.2 `:1047`, §7 `:1086`), so the third case's premise — that cancelling on breach saves the remaining spend — is not merely unmet but **meaningless**: by the time the figure exists the turn is over. Rather than let a transport quietly pass a case it cannot honour, the capability is named and the contract splits: over ACP the per-run cap is a live client-side brake (`enforce_breach`, D69), over CLI the live brake is `--max-budget-usd`, enforced server-side by the CLI itself during the turn (D83). Maintainer accepted at the blueprint gate. |

All three blocking findings are answered; implementation is unblocked.

## Files to Change

Per-task file sets are listed under each task and are what the fact-check step intersects; this
table is the union.

| File | Action | Why |
|---|---|---|
| `crates/htui-core/seeds/agent_claude_cli.json` | CREATE | D79's row |
| `crates/htui-core/seeds/agent_claude.json` | UPDATE | D79 drops its `settings.cli` block |
| `crates/htui-core/src/model/agent.rs` | UPDATE | third `include_str!`; `seed_rows_match_ana4_5_3` gains the `cli` row's expectations |
| `crates/htui-core/src/fixtures.rs` | UPDATE | the demo fixture follows the seeds (§5.3's third note) |
| `crates/htui-store/src/pg/mod.rs` | UPDATE | D88 `seed_missing_agents` |
| `crates/htui-store/src/pg/demo.rs` | UPDATE | the demo reset deletes seeded rows by name (`demo.rs:9`, `:141`) — a third name |
| `crates/htui-agent/src/cli/mod.rs` | CREATE | supervisor, session task, stdin follow-ups, cancel, banner |
| `crates/htui-agent/src/cli/claude.rs` | CREATE | §6.2 mapper |
| `crates/htui-agent/src/lib.rs` | UPDATE | `pub mod cli;` |
| `crates/htui-agent/src/registry.rs` | UPDATE | register `cli/claude_stream_json`; D87 rename |
| `crates/htui-agent/src/error.rs` | UPDATE | only if a new named cause is needed (e.g. a malformed envelope) |
| `crates/htui-agent/src/driver.rs` | UPDATE | D90's `SessionSpec.budget_micros` |
| `crates/htui-agent/src/conformance.rs` | UPDATE | D80's capability arms |
| `crates/htui-agent/tests/cli_conformance.rs` | CREATE | third binding of one `CASES` list |
| `crates/htui-agent/tests/cli_map.rs` | CREATE | fixture → `insta` snapshots of the mapper |
| `crates/htui-agent/tests/cli_live.rs` | CREATE | T55/T56 probes and the no-surviving-children assertion |
| `crates/htui-agent/tests/fixtures/claude_stream_json*.jsonl` | CREATE | recorded transcripts |
| `crates/htui-agent/tests/{driver_contract,acp_driver,auth_live}.rs`, `crates/htui/tests/auth.rs` | UPDATE | D87 rename call sites |
| `crates/htui-agent/tests/extensibility.rs` | UPDATE | the `R-AGT-5` sweep now has a second transport to measure |
| `crates/htui/src/agent_worker.rs` | UPDATE | D87 rename (2 sites); D83's `--max-budget-usd` cap hand-off if it is read here |
| `crates/htui/tests/chat_live_cli.rs` | CREATE | the milestone's live proof |
| `crates/htui/src/ui/tabs/settings/agents.rs` | UPDATE | D89's re-ranking and its rewritten comment block |
| `crates/htui/tests/settings.rs` | UPDATE | D89's width cases |
| `crates/htui/tests/snapshots/*`, `crates/htui-agent/tests/snapshots/*` | UPDATE | Settings agents table gains a row and re-ranks; banner snapshots |
| `README.md` | UPDATE | what the CLI row is, when to prefer it, and what it cannot do |
| `HANDOFF.md`, `.claude/prds/mod-2-agent-driver-chat.prd.md` | UPDATE | phase note; milestone row |

---

## Tasks

Ordering is the build order. **T49→T49b, T50+T51, and T55→T56 are the three independent fronts**;
everything after T54 is serial on them.

### T49: The `claude-cli` row (D79, D88) — independent

- **Action**: write `agent_claude_cli.json`; drop `settings.cli` from `agent_claude.json`; extend
  `seed_rows` and its ANA-conformance test (which currently asserts `rows.len() == 2` and
  `transport == Acp` for **every** row — `model/agent.rs:196-213`, so it fails loudly and correctly
  until updated); follow the demo fixture and the demo reset; add `seed_missing_agents`.
- **Mirror**: `agent_agy.json` for the document shape; `seed_rows_match_ana4_5_3` for the assertion
  style.
- **Files**: `crates/htui-core/seeds/agent_claude_cli.json`,
  `crates/htui-core/seeds/agent_claude.json`, `crates/htui-core/src/model/agent.rs`,
  `crates/htui-core/src/fixtures.rs`, `crates/htui-store/src/pg/mod.rs`,
  `crates/htui-store/src/pg/demo.rs`, plus the Settings snapshots the new row moves.
- **Validate**: `cargo test -p htui-core seed_rows`; `cargo test -p htui-store --all-features`;
  `cargo test -p htui --features demo,test-support settings`.

### T49b: The Settings width re-ranking (D89) — independent of T49's data, same snapshots

- **Action**: `name` → `Constraint::Min(10)`, `on this box` → `Constraint::Length(13)`; rewrite the
  `agents.rs:1021-1066` comment block to record the new ranking (`name` first, `on this box` the
  donor) instead of D76's; re-accept the width-sensitive snapshots.
- **Mirror**: the comment block itself — D76's ranking is written as a ranking with its donors and
  their costs named, and the replacement owes the same.
- **Files**: `crates/htui/src/ui/tabs/settings/agents.rs`, `crates/htui/tests/settings.rs`,
  `crates/htui/tests/snapshots/*`.
- **Validate**: `cargo test -p htui --features demo,test-support settings`. Three checks, and the
  first is the one that matters: **`claude-cli` renders whole at the 100-column guard width**;
  `unauthenticated` renders `unauthenticat` and the snapshot records that as accepted, not
  accidental; a 160-column render shows `name` absorbing the slack.
- **Note**: serial with T49 — both re-accept `crates/htui/tests/snapshots/*`.

### T50: The CLI supervisor — `src/cli/mod.rs` (D81, D83, D84)

- **Action**: `ClaudeStreamAdapter: TransportBuilder`; a session task owning the child through a
  `ChildGuard`; the §4.4 invocation assembled from `agent.launch` + `settings.cli`
  (`--permission-mode` from the row, `extra_args` appended, `--add-dir` per `SessionSpec.extra_dirs`,
  `--session-id` from D84's mint, `--max-budget-usd` from `SessionSpec.budget_micros` per D83/D90);
  an NDJSON line reader over stdout
  into `DriverEvent`s via T51's mapper; stderr captured to the run log as the ACP path does;
  `send_follow_up` writing one NDJSON user message per line to stdin; `cancel` per D81; the banner
  per D84; a `done` per turn, and a follow-up refused before it.
- **Mirror**: `acp/mod.rs`'s session-task shape; `probe.rs`'s `ChildGuard`/`run_bounded` for child
  ownership on every exit path.
- **Files**: `crates/htui-agent/src/cli/mod.rs`, `crates/htui-agent/src/lib.rs`,
  `crates/htui-agent/src/error.rs`, `crates/htui-agent/src/driver.rs` (D90's field),
  `crates/htui/src/agent_worker.rs` (D90's caller: the cap already computed at `:2307-2312` is
  handed to `SessionSpec`).
- **Validate**: `cargo test -p htui-agent --features test-support`; `cargo clippy --target
  x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features`.

### T51: The `claude` stream mapper — `src/cli/claude.rs` (D82, D85, D86)

- **Action**: every row of §6.2's table, decoded against a wildcard-armed shape: `assistant` text
  and `stream_event` `text_delta` → `assistant_text` coalesced on `message.id`; `thinking` →
  `thought`; `tool_use` → `tool_call` with `tool_kind` derived from the tool **name** (`Read`→`read`,
  `Edit`/`Write`→`edit`, `Bash`→`execute`, else `other`); `tool_result` joined on `tool_use_id`;
  `result` → `usage` (D86) + `error` + `done`; `permission_denials` → `permission_answer` (D85);
  `system/init`, hook events, `rate_limit_event` and plugin events → `other`, with `rate_limit_event`
  additionally driving the quota latch.
- **Mirror**: `acp/map.rs` — mapping only, no process type imported, unit-testable from one line.
- **Files**: `crates/htui-agent/src/cli/claude.rs`, `crates/htui-agent/tests/cli_map.rs`,
  `crates/htui-agent/tests/fixtures/claude_stream_json*.jsonl`.
- **Validate**: `cargo insta test -p htui-agent --test cli_map` (fixtures land in T56).
- **Note**: T50 and T51 share `src/cli/` and the `lib.rs` line. **They are not independent of each
  other** and run serial or as one agent's pair; they *are* independent of T49.

### T52: Registration and the rename (D87)

- **Action**: `DriverFactory::with_acp` → `production`, registering `acp` and
  `cli/claude_stream_json`; update the eight call sites; add to `extensibility.rs` an assertion on
  the **production** factory that `adapter_ids() == ["acp", "cli/claude_stream_json"]` — two
  adapters for three rows is the number `R-AGT-5` measures. The existing assertion at
  `extensibility.rs:177` is about a test-local factory holding only `cli/fake` and stays as it is.
- **Files**: `crates/htui-agent/src/registry.rs`, `crates/htui-agent/tests/extensibility.rs`,
  `crates/htui-agent/tests/{driver_contract,acp_driver,auth_live}.rs`,
  `crates/htui/src/agent_worker.rs`, `crates/htui/tests/auth.rs`.
- **Validate**: `cargo test --workspace --all-features`.

### T53: Capability arms in the conformance suite (D80)

- **Action**: give the three capability-dependent cases their second arm, reading `driver.caps()`;
  document each arm next to its assertion the way `fake.rs` documents the five harness rules.
- **Files**: `crates/htui-agent/src/conformance.rs`.
- **Validate**: `cargo test -p htui-agent --features test-support --test fake_conformance --test
  acp_conformance` — **both existing bindings must stay green unchanged**, which is what proves the
  arm did not weaken the ACP side.

### T54: The CLI conformance binding

- **Action**: `tests/cli_conformance.rs` — a `CaseHarness` whose agent side writes raw
  stream-json NDJSON, importing no adapter type, driving the real supervisor over a pipe. The row
  comes from `seed_rows`, filtered to `claude-p`.
- **Mirror**: `acp_conformance.rs` end to end, including its duplex-buffer note.
- **Files**: `crates/htui-agent/tests/cli_conformance.rs`.
- **Validate**: `cargo test -p htui-agent --features test-support --test cli_conformance` — all 15
  `CASES` names reported, none added.

### T55: Cancellation probe (D81) — live, independent, runs first among the live pair

- **Action**: an `#[ignore]`d test that starts a real `claude -p` turn and cancels it, once with
  SIGINT and once with SIGTERM, recording the resulting terminal envelope and exit status. Its
  finding fixes D81's middle step and §11.14's third item; the transcript is committed as a fixture.
  **It drives the binary directly, not through T50's supervisor** — that is what makes it a probe
  rather than a test of code the probe is supposed to inform, and what makes it independent of the
  `src/cli/` front.
- **Files**: `crates/htui-agent/tests/cli_live.rs`,
  `crates/htui-agent/tests/fixtures/claude_stream_json_cancel.jsonl`.
- **Validate**: `cargo test -p htui-agent --features test-support --test cli_live -- --ignored
  --nocapture`; asserts no surviving `claude` process (criterion 11's CLI half).

### T56: Thinking and usage probe (D82, D86) — live, independent of T55's finding

- **Action**: one real turn with `--include-partial-messages` and thinking enabled, one that runs a
  subagent (so `modelUsage` and `result.usage` visibly disagree and D86's choice is evidenced), and
  one that trips `--max-budget-usd` at a near-zero value (D83's server-side arm). Transcripts
  committed as fixtures; §11.14's fourth item closed.
- **Files**: `crates/htui-agent/tests/cli_live.rs` (shared with T55 — **serial with T55**),
  `crates/htui-agent/tests/fixtures/claude_stream_json_{thinking,subagent,budget}.jsonl`.
- **Validate**: as T55.

### T57: The live chat (criterion 9's CLI half)

- **Action**: `crates/htui/tests/chat_live_cli.rs` — a whole chat streamed into the store through
  the production runtime over the CLI row, mirroring `chat_live.rs`. Asserts the banner names the
  three missing capabilities, that `run_step.usage` equals the sum of the step's `usage` rows
  (criterion 7), and that the process tree is gone at the end.
- **Files**: `crates/htui/tests/chat_live_cli.rs`, `crates/htui/tests/snapshots/*`.
- **Validate**: `cargo test -p htui --features demo,test-support --test chat_live_cli -- --ignored
  --nocapture` (burns model tokens against the maintainer's credential).

### T58: Close-out (serial, last)

- **Action**: `README.md` gains the CLI row's purpose and its declared gaps; the §11.14 answers land
  as a table in this plan and in the eventual `docs/decisions/mod/mod-2.md`; `HANDOFF.md` gains the
  phase note **including D79's and D88's ANA-4 amendments**, per the milestone-5 precedent; the
  PRD's milestone 8 row goes `complete` and its open-questions list loses the two closed §11.14
  items.
- **Files**: `README.md`, `HANDOFF.md`, `.claude/prds/mod-2-agent-driver-chat.prd.md`, this plan.
- **Validate**: `pwsh`/`bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.

---

## Validation

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --target x86_64-pc-windows-msvc -p htui-agent --all-targets --all-features
USERNAME=htui-ci HTUI_TEST_DATABASE_URL=postgres://postgres:htui@localhost:5439/postgres \
  cargo test --workspace --all-features

# T55/T56, explicitly (spawns the real CLI; T56 burns model tokens)
cargo test -p htui-agent --features test-support --test cli_live -- --ignored --nocapture

# T57, explicitly (burns model tokens against the maintainer's credential)
cargo test -p htui --features demo,test-support --test chat_live_cli -- --ignored --nocapture
```

---

## Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| D80's capability arm quietly weakens criterion 1 into "every transport passes the cases it feels like" | Medium | High | The arm **asserts the negative**, it does not skip: a CLI transport that started emitting `permission_request` rows would fail the case. T53's validation requires both existing bindings green **unchanged**, which is the check that the ACP side lost nothing |
| The live cancellation semantics differ from §4.4's hypothesis and D81's design is wrong | Medium | Medium | T55 runs **before** the `cancel` code is written and the plan says so; the finding amends D81 rather than the code being written twice. This is the milestone-6 D58 pattern (the `--uid=` finding), which is why it is scheduled this way |
| `--bare` or `--include-partial-messages` changes shape in a `claude` release between now and MOD-4 | Medium | Medium | The fixtures are the regression net (§8 test strategy 3): an unknown envelope lands in `other` and the snapshot shows it, rather than the build breaking. `system/init.capabilities[]` is the documented feature-detection channel and is recorded in the banner |
| D88's top-up resurrects a row the maintainer deleted | Low | Low | Stated at CONFIRM with its alternative; MOD-23's model is `enabled = false`, not deletion. `ON CONFLICT (name) DO NOTHING` means it never overwrites an edited row |
| Two `claude` rows confuse the Settings table, which milestone 7's D76 left with **no width slack** | — | — | **Confirmed and priced, not a risk**: `name` is `Constraint::Length(8)` (`agents.rs:1069`) and the eight columns share exactly 91 at the 98-column section, every other column at its own floor. D89 re-ranks rather than renames, and `on this box` pays 2. T49b's snapshot is the check |
| MOD-21's `unauthenticated` cell reads `unauthenticat` after D89, and that word is a whole login flow (`agents.rs:1036-1040`) | Certain | Low | Named in D89 as the accepted price with the maintainer's override left open. It is a *status* cell, where the two columns above it in D76's ranking carry a selection coordinate and an exhaustion warning; the `a` key is still offered on the row, and the truncation is legible. If it proves wrong in use it is a one-line re-rank, not a redesign |
| A CLI session's stdin close races its last stdout lines and truncates a turn | Medium | High | The supervisor drains stdout to EOF after the close before reporting `done`; the milestone-3 defect "a stream ending before its `done` was recorded as a finished turn" (`682a423`) is the precedent and its test is the model |
| Windows: `claude` resolves to a `.cmd` shim `CreateProcess` refuses, exactly as §4.4 found for `claude-agent-acp` | High | Medium | `which` with `PATHEXT` is already the resolver (`tools::resolve`), and this is compile- and lint-checked from Linux only; the runtime fact is **MOD-16's**, and the phase note will say so rather than claiming a green it did not earn (TOOL-2's lesson) |

---

## Verified claims

Checked against the tree at `f5b37ad` on 2026-09-10, before CONFIRM. The fact-check step (skill
step 3.5) extends this table before the maintainer sees the plan.

| Claim | Verdict | Evidence |
|---|---|---|
| `DriverFactory` keys a CLI row as `cli/<settings.cli.stream>` | true | `registry.rs:122-130` |
| `caps_from` already returns §4.3's `DriverCaps` triple for `Transport::Cli` | true | `registry.rs:159-171` |
| `CliSettings { stream, permission_mode, extra_args }` already exists and parses | true | `launch.rs:358-374` |
| `QuotaSource::{CliRateLimitEvent, CliStatusLine}` already exist | true | `htui-core/src/model/quota.rs` `str_enum!` arms |
| `UsageScope::ModelUsage` already exists and is documented as CLI-only | true | `launch.rs:399-410` |
| The probe already declines tier 2 for a non-`acp` row | true | `probe.rs:1444-1448`, doc step 6 at `probe.rs:1315` |
| The chat tab's banner is computed from `DriverCaps`, not `agent.transport` | true | `chat/mod.rs:320-333` |
| `CASES` holds 15 names and no transport is named in `conformance.rs` | true | `conformance.rs:131-147`, module doc `:1-7` |
| `conformance.rs` has **no** capability awareness today | true | grep for `caps`/`skip` over `conformance.rs` returns only an unrelated doc line at `:105` |
| `seed_rows` reads exactly two `include_str!` documents | true | `model/agent.rs:130-134` |
| `seed_rows_match_ana4_5_3` asserts `len() == 2` and `Transport::Acp` for every row | true | `model/agent.rs:196-213` — T49 must update it |
| `PgStore` seeds agents only when the table is empty | true | `model/agent.rs:107` doc; `pg/mod.rs:251` `seed_if_empty_as` |
| `DriverFactory::with_acp` has 8 call sites | true | grep: `registry.rs:65` (def) + `auth_live.rs:820,944`, `driver_contract.rs:426,438`, `acp_driver.rs:664`, `agent_worker.rs:365,5353`, `auth.rs:263` |
| `claude` on this box is 2.1.267 and carries every §4.4 flag | true | `claude --version`; `claude --help` greps for `--bare`, `--max-budget-usd`, `--input-format`, `--output-format`, `--include-partial-messages`, `--permission-mode`, `--session-id`, `--resume`, `--add-dir`, `--forward-subagent-text`, `--verbose` all hit, 2026-09-10 |
| D60 assigns the CLI **row** to milestone 8 | true | `.claude/plans/mod-2-agy-acp.plan.md:105` |
| `--max-budget-usd` was deferred to this milestone by milestone 7 | true | `.claude/plans/mod-2-quota-caps.plan.md:403` |
| MOD-11 is not required for milestone 8 | true | `ANA-4.md:550-553`, `:1043-1046`; PRD `:87-89`, `:120-121` |
| ANA-4 §5.3 currently puts the `cli` block **inside** the `acp` `claude` row | true | `ANA-4.md:928-929`, `:658-662`; `seeds/agent_claude.json` — hence D79's amendment |
| Highest decision id in use is D78; highest task id is T48 | true | grep over `.claude/plans/mod-2-*.plan.md` |
| `ChildGuard` lives in `probe.rs` | **false — corrected** | it is `launch.rs:734` (`pub(crate) struct ChildGuard`); `run_bounded` is `probe.rs:982`. The Patterns table now names both correctly |
| `AcpAdapter::build` is at `acp/mod.rs:326-333` and ignores `on_box` | **false — corrected** | it is `acp/mod.rs:548-559` and delegates to `AcpDriver::from_row_with_probe(agent, on_box, caps)`; the `_on_box` form was pre-D58 and the agy plan's own claim is stale |
| `tools::resolve` is the `PATHEXT`-correct resolver | partly | `tools.rs:68` is `resolve(discovery, cwd) -> ToolMap`; the `which` calls are `probe.rs:500` (`which_in`) and `launch.rs:829` (`which`, on `spawn_blocking`). The Windows risk row is about `launch.rs:829`, which is the path a CLI spawn takes |
| `fake.rs` documents five harness rules | true | `fake.rs:9-27`, numbered 1–5 |
| `enforce_breach` is shared by `pump` and `run_turn` (D69) | true | `record.rs:1705`; imported in `agent_worker.rs:45` as `enforce_cap_breach` |
| `AgentSession::cancel` takes a grace window | true | `driver.rs:425` — `cancel(&mut self, grace: Duration)`; D81's grace step has somewhere to live |
| The chat path already computes the per-run cap in micros before the turn | true | `agent_worker.rs:2307-2312`, `RunCap { micros, grace }` — the figure D83 needs, hence D90 |
| `SessionSpec` has no budget field | true | `driver.rs:252-273` lists eleven fields, none of them a cap |
| `TransportBuilder::build` takes no run context | true | `registry.rs:38-43` — `(agent, on_box, caps)` |
| The Settings `name` column is 8 wide | true | `settings/agents.rs:1069`; headers at `:1010-1017` confirm column 1 is `name` |
| A `Constraint::Length(128)` `name` column is achievable | **false — instruction amended to `Min(10)`** | the section draws 98 columns at the 100-column guard width (`agents.rs:1021-1022`, `tests/settings.rs:49`); minus 7 `column_spacing` gaps the eight columns share **91**, and today's widths sum to exactly 91 (8+9+12+6+21+7+13+15). 128 for one column exceeds the whole table |
| Every other column is already at its own floor, so widening `name` must take from one | true | `transport` 9 / `models` 6 / `enabled` 7 at their headers; `billing` 12 = `subscription`; `default` 21 = `gemini-3.7-flash-high`; `quota` 13 = `100% to 09-08`; `on this box` 15 = `unauthenticated` — each stated with its longest string at `agents.rs:1032-1051` |
| `extensibility.rs` already asserts production `adapter_ids` | **false — corrected** | `:177` asserts `["cli/fake"]` on a test-local factory; T52 adds the production assertion rather than editing that one |
| **Task independence** (file-set intersection, step 3.5) | verified | T49 (`htui-core/seeds`, `htui-core/src/model/agent.rs`, `fixtures.rs`, `htui-store/src/pg/*`) ∩ T50+T51 (`htui-agent/src/{cli/*,lib,error,driver}.rs`, `agent_worker.rs`) = ∅; T55+T56 (`htui-agent/tests/cli_live.rs`, fixtures) ∩ both = ∅. **T50 ∩ T51 ≠ ∅** (`src/cli/`, `lib.rs`) → serial pair, stated in T51. **T50 ∩ T52 ≠ ∅** (`agent_worker.rs`) → T52 runs after T50, which the build order already has. So the parallel set is exactly **{T49, T50+T51, T55→T56}** |

---

## Acceptance

- [ ] All tasks complete
- [ ] `CASES` still holds 15 names, and three bindings report every one
- [ ] `adapter_ids()` is 2 for 3 registry rows (`R-AGT-5`)
- [ ] §11.14's CLI cancellation and thinking-block items answered from a live run, recorded as
      fixtures
- [ ] Criterion 7 holds over the CLI transport (`run_step.usage` = the sum of its `usage` rows)
- [ ] Criterion 11's CLI half holds on Linux; the Windows half is stated as MOD-16's, not claimed
- [ ] The chat tab banner names `permission_request`, `edit_proposal` and `plan` as unavailable
- [ ] `claude-cli` renders whole in the Settings `name` column at the 100-column guard width, and
      `name` absorbs the slack at wider ones (D89)
- [ ] `rust-reviewer` gate run over the full change set, findings applied or deferred with the
      maintainer
- [ ] Validator green; PRD milestone 8 row `complete`; `HANDOFF.md` phase note carries D79/D88's
      ANA-4 amendments

# ANA-36 - Further MCP tools for agents

> **Scope note:** Survey which further tools `htui`'s MCP server should give the agents it launches,
> beyond the eight it ships and the ones the 2026-10-15 review already named (read tools, search,
> `item_create` drafts, the question tool of MOD-75, the planner's tools of ANA-33). Three inputs: what
> comparable harnesses, trackers and agent frameworks expose to their agents; which tools would replace
> work agents do badly today, read from the stored transcripts; and, for each candidate, its scope under
> `R-MCP-1` and its line under `R-ID-6`. Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md`,
> `docs/REQUIREMENTS.md`, `docs/requirements-review-2026-10-15.md` (the item's origin, `R-MCP-2` at
> `:196-203`), `docs/htui-mcp.md` and MOD-11's decisions (`docs/decisions/mod/mod-11.md`). Best read
> beside ANA-33 (planner tools; on an unmerged branch, see §1 caveats) and ANA-34 (coordinator tools).
>
> **Requirements addressed:** `R-MCP-2`, `R-MCP-1`, `R-ID-6`. Touched at their seams: `R-MCP-3` and
> `R-MCP-4` (`command_run`), `R-PRM-1`/`-2`/`-3` (what the prompt already carries, trims), `R-ENT-8`
> (status requests), `R-ENT-9` (links), `R-ENT-12` (attachments), `R-ENT-14`/`-15` (requirement drafts,
> citations), `R-ID-5` (skill text inlined), `R-ID-7` and `R-SEC-2`/`-3` (scrub, secrets),
> `R-SKL-1` (skill resources), `R-TUI-11` (waiting on you), `R-TUI-17` (talk-it-through draft),
> `R-AGT-11` (the htui agent).
>
> **Status (2026-10-18): concluded; the maintainer accepted all thirteen §8 answers as recommended.** No
> implementation code is written here. Verdict (§7): of 36 candidate tools, **11 are refuted** by the
> skeptic pass and go to §10; **15 are already owned** by an existing item (MOD-112, MOD-75, MOD-27,
> MOD-135, MOD-136, MOD-118/119, MOD-121, MOD-113, ANA-38, and MOD-140 on the ANA-33 branch), and ANA-36
> hands each owner its design input instead of re-proposing it; **the 10 unowned survivors** become seven
> MOD items, **MOD-151** (P1) to **MOD-157** (P7): tell agents about the tools, an async command queue
> with stored logs, structured test results with named command presets, `item_status` as a typed request,
> link and citation proposals that stay pending until a person accepts them, `document_patch`, and
> attachment tools. The largest change of principle is in **MOD-155** (P5): an agent's `blocked_by` link
> is live today and feeds auto mode's picks, which `R-ID-6` does not allow.

---

## 1. Context and problem statement

`R-MCP-2` lists the tools `htui` gives an agent: `item_link`, `item_status`, `document_write`,
`note_add`, `box_profile`, `command_run`, `spawn_subagent`, read tools, semantic search, `item_create`,
a question that parks the step, and the planner's tools, and ends "ANA-36 surveys further tools"
(`docs/REQUIREMENTS.md:514-519`). The review that added that sentence asked for "an ANA to research other
kinds of tools agents should get" (`docs/requirements-review-2026-10-15.md:196-203`). Eight tools ship
today under MOD-11; most of the rest already have an owner (§2.3). So the question is not "what does
`R-MCP-2` miss" in the abstract, but: **which tool would stop an agent from doing a job badly, that no item
owns yet, and where does each sit against `R-MCP-1` (write in the run, read beyond it) and `R-ID-6`
(model text is a display-only label or a draft a person accepts; nothing that moves state reads
unaccepted model text)?**

An example of the kind of answer this item gives: today an implementer that runs the test suite through
`command_run` waits up to 30 minutes for one call, gets back the last 64 KiB of output, and has no way to
read the rest except running the suite again. In the stored transcripts, agents re-ran a failed
`cargo test` with no edit in between 978 times, mostly to read the output differently (§4). A
`command_output_read` tool over a stored log (**MOD-152** (P2)) and a structured result (**MOD-153** (P3)) replace that work; both
are deterministic, so `R-ID-6` has nothing to say about them, and both only read the run's own rows, so
`R-MCP-1` is met.

**Method.** A scripted sweep (`/tmp/ana36/sweep.js`, output `/tmp/ana36/out/sweep.json`) ran seven
readers in parallel: the htui MCP surface in code; requirements and sibling items; coding-agent
harnesses; trackers, CI, memory servers and multi-agent frameworks; an aggregate pass over every stored
transcript; and two episode readers over the older and newer halves of the top-level sessions. A
catalogue step merged their ideas into 36 candidates, 20 rejected ideas and 17 cross-cutting rules; a
critic added three candidates and listed missing angles; a skeptic then checked every candidate against
code and evidence and returned a verdict (refuted, problems, corrections). **The skeptic's verdicts are
binding here**: a refuted candidate goes to §10 or is folded into the tool it duplicates, and its
corrections to owner, scope, value and `R-ID-6` line are applied in §5. The code claims in §2 and the
evidence behind §7 were re-opened for this doc (Gortex reads, `git show`), and corrected where they did
not hold.

**Caveats on the evidence:**

- **No transcript is an htui-launched session.** All 2,833 stored transcripts (2026-09-18 to 10-18) are
  Claude Code sessions run directly on the htui repository against markdown `HANDOFF.md`, and none ever
  called an `mcp__htui__*` tool. They show what agents do without htui's tools, not how they use them.
  Every count about HANDOFF, DECISIONS, ID minting and summary-table recounts measures the markdown
  workflow htui replaces with store rows, so those counts are **upper bounds** on demand.
- **Two sources of friction are this dev setup's, not htui's.** The Gortex enforcement hook denied 4,203
  calls (most tool errors), including 355 greps over cargo output; and Claude Code's auto-mode classifier
  denied 94 calls, 30 of them on credentials. An htui step runs under neither, so neither counts as
  evidence for an htui tool on its own.
- **Counts are regex heuristics** over tool-use inputs and results (scripts under `/tmp/ana36/*/`).
  Two variants of the planning-doc read count gave 8,380 and 9,859; "question at turn end" gave 152/731 or
  136/495 depending on what counts as a human prompt. Some sleeps and re-runs were deliberate (re-checks
  under load), so waste figures are upper bounds.
- **Cargo wall-clock and token cost were not measured.** No reader computed tool-use to tool-result
  durations for cargo runs, or cost per pattern. The value ranking of **MOD-152** (P2) (async commands, stored logs) and
  **MOD-153** (P3) (structured results) therefore rests on **call counts only**.
- **Question waits include the maintainer being away.** AskUserQuestion wait times are timestamp
  differences; one deliberate 37-question requirement interview (`93e329d7`) inflates the newer-half total.
- **The harness survey has gaps.** Cline's source was not readable (names from docs); Amp's names come
  partly from a third-party system-prompt copy; Copilot's `report_progress` is community-reported only;
  Linear's MCP tool names come from directories and Atlassian's from a search summary; GitHub's issue
  dependency tool name was not confirmed. Not surveyed at all: Continue, Windsurf/Cascade, Kiro,
  OpenCode/Crush, Augment and Warp.
- **ANA-33 is not on this branch.** ANA-33 was concluded on branch `hr/ANA-33` (commits `cf41f323`,
  `7a81dcd2`), which is not merged into `main` or `hr/ANA-36` (`git merge-base --is-ancestor` fails).
  `docs/ANA-33.md` does not exist on this HEAD, and MOD-137..143 exist only there. Every ANA-33 claim below
  is cited as `origin/hr/ANA-33` and was read with `git show origin/hr/ANA-33:docs/ANA-33.md` or
  `:HANDOFF.md`. That branch was cut before ANA-34 concluded; its HANDOFF drops MOD-133..136, so the merge
  needs care.
- **Not read:** `channel.rs` (handshake), the judge-session path, the body of
  `persona_keeps_command_run`, and the PgStore `step_scope` bodies behind the unleased-fence refusal.
  Facts about them come from `docs/htui-mcp.md` and doc comments.
- **Open `R-ID-6` readings** the skeptic could not close from the text: whether another agent's note,
  returned to an agent by `item_read`, counts as "a prompt reads unaccepted model text" (§8 Q9); and
  whether a skill with `glob` activation can activate mid-session (unverified; if not,
  `skill_resource_read` reaches only skills selected before the session starts, §7.2).

## 2. The current htui MCP surface

### 2.1 The eight shipped tools

One fixed table of eight tools, in pinned `tools/list` order (`crates/htui-mcp/src/tools/mod.rs:45-54`),
dispatched by name (`:75-91`). Each tool has a predicate that decides per session whether it is
advertised; a tool that is not advertised is left out of `tools/list` and refused as unknown
(`crates/htui-mcp/src/host.rs:119-124, 133-145`). Every argument object except `permission_prompt`'s
refuses unknown keys, so an agent cannot name a run, item, box or project (MOD-11 I-1).

| Tool | Arguments | Advertised when | Scope (`R-MCP-1`) | Store effect | Evidence |
|---|---|---|---|---|---|
| `box_profile` | none | always | the session's box | read only: the stored probe, as the prompt's box section renders it | `tools/box_profile.rs:12-45`, predicate `:24` |
| `document_write` | `body`, `title?` | the scope has an item **and** an output kind | the run's item, the phase's `output_kind` | new document version per call, fenced, scrubbed | `tools/document.rs:15-69`, predicate `:31` |
| `note_add` | `body` (at most 16384 bytes) | the scope has an item | the run's item | one `item_note` with `via_step_id`, fenced, scrubbed | `tools/note.rs:16-76` |
| `item_status` | `status`, `reason`, `resolution?` | the scope has an item | the run's item | a note `status request: …`; never moves the status (I-4) | `tools/status.rs:1-82` |
| `item_link` | `op` add/remove, `to` key, `kind` | the scope has an item | `from` is the run's item; `to` resolves in the run's project only | `propose_link` upserts a **live** row; remove withdraws only this run's | `tools/link.rs:55-127`; `crates/htui-store/src/pg/write.rs:7104-7166` |
| `search_concepts` | `query`, `types?`, `statuses?`, `limit?` 1-20 | the host has a concept index | the run's project | read only: key, type, score, snippet (at most 280 chars, unscrubbed) | `tools/search.rs:16-112`, predicate `:68`; `crates/htui-mcp/src/search.rs:85-107`; `docs/htui-mcp.md:117-126` |
| `command_run` | `class` build/test/run, `command`, `cwd?`, `timeout_secs?` 1-1800 | the phase's command queue is exposed | the step's box queue, cwd under `scope.cwd` | a `command_run` row; blocks until exit; returns a scrubbed 64 KiB tail | `tools/command.rs:40-60, 330-377`, predicate `:140`; `crates/htui-orch/src/verify.rs:522` |
| `permission_prompt` | `tool_name`, `input`, `tool_use_id?` | the transport is `claude-cli` | the session's own permission requests | relays to a person over the prompt bridge; hidden from the model by the CLI | `tools/permission.rs:16-63`, predicate `:21` |

Paths without a crate prefix are under `crates/htui-mcp/src/`.

### 2.2 How scope, writes and exposure work

- **Scope is a struct the engine builds.** `ToolScope` holds `run_id`, `step_id`, `project_id`,
  `item_id` (`None` in a fresh chat), `box_id`, `user`, `fence`, `output_kind`, `hostname`,
  `command_queue`, `cwd` and `transport`; it has **no workspace field and no `extra_dirs`**
  (`crates/htui-orch/src/tools.rs:21-49`). So `R-MCP-1`'s wider read scope ("workspace items, documents
  and requirements") is not implemented: `item_link` and `search_concepts` resolve in the run's project.
- **Writes are fenced store calls, not `StoreRequest`s.** The host captures the worker's writer and
  calls fenced `WriteStore` methods (`write_step_document`, `add_step_note`, `propose_link`,
  `withdraw_link`). `WriteStore::cite` exists but takes no fence and is not on `WorkerStore`
  (`crates/htui-core/src/store/traits.rs:1775-1789`), so an agent citation has no fenced path today.
- **Reads the store already has but no tool exposes.** `WorkerStore` carries `item`, `documents`,
  `document`, `documents_of_kinds`, `runs`, `run_steps`, `step_events`, `step_commits`, `notes`,
  `resolve_inputs` and `command_runs` (`crates/htui-core/src/store/worker.rs:141-204`); `ReadStore` adds
  `items(scope, filter)` and `links(id, hops)` (`traits.rs:99-103`). Most read tools are a lift, not new
  queries.
- **`command_run` exposure.** `CommandQueue::exposed`: `off` never, `always` always, `fan_out_only` when
  the phase fans out to more than one agent or the item carries `heavy_build`
  (`crates/htui-core/src/model/kind.rs:53-65`). While it is exposed, the engine prepends one
  `reject_once` per heavy prefix (`cargo test`, `make`, `go test`, …) so the harness shell refuses them,
  a prefix nudge and not a fence (`crates/htui-orch/src/engine.rs:6525-6541`; `kind.rs:78-98`). Chats
  always get `command_queue: false` (`crates/htui/src/agent_worker.rs:1112, 2443`). One call may be in
  flight per session (`tools/command.rs:52`).
- **Approval.** Every advertised htui tool is pre-approved under the default `ask` policy except
  `ASKING_TOOLS = [command_run, permission_prompt]` (`crates/htui-orch/src/tools.rs:91-132`).
- **The protocol is minimal.** `initialize` answers `capabilities: {tools: {}}` and `serverInfo` only:
  no `instructions`, resources, prompts or `listChanged` (`crates/htui-mcp/src/protocol.rs:350-362`).
  A result is one text block; there is no `structuredContent` or `outputSchema`. Cancellation aborts the
  in-flight call (`protocol.rs:336-347`).
- **Agents are told about two tools.** The prompt names only `document_write` (the output trailer,
  `crates/htui-core/src/prompt/render.rs:636-646`) and `command_run` (`COMMAND_QUEUE_TEXT`,
  `crates/htui-core/src/prompt/defaults.rs:26`). That text says to route "verification" through
  `command_run`, which refuses class `verify` as the orchestrator's (`tools/command.rs:48-49`).
- **Two seams that break `R-ID-6` or hide output.** An agent's `item_link add` is live at once
  (`pg/write.rs:7137-7146`), and `ready_items` excludes items with a live `blocked_by` to an open item
  (`crates/htui-store/src/pg/read.rs:2097-2100`), so an unaccepted agent link changes what auto mode can
  pick; `queue_rows` also lists those blockers (`read.rs:2364-2368`). And `item_status` requests reach a
  person only as the queue row's latest note (`read.rs:2357-2362`) and the item's notes thread; no code
  reads "status request" notes as requests (Gortex text search: only `status.rs` and its test).

### 2.3 What `R-MCP-2` names and who owns it

| `R-MCP-2` part | State | Owner |
|---|---|---|
| `item_link`, `item_status`, `document_write`, `note_add`, `box_profile`, `command_run` | shipped | MOD-11 (done) |
| semantic search | shipped, project-only | MOD-11; widening MOD-112 (2), (4) |
| read tools for items, documents, requirements | missing | MOD-112 (1) (`HANDOFF.md:689-696`) |
| `item_create` as a draft | missing | MOD-112 (3); draft store MOD-135 (`HANDOFF.md:424-430`) |
| question that parks the step | missing | MOD-75 (`HANDOFF.md:491-501`) |
| planner's tools | missing | MOD-140 (`origin/hr/ANA-33:HANDOFF.md:875-883`) |
| `spawn_subagent` | missing | MOD-27, narrowed to consult mode (`origin/hr/ANA-33:HANDOFF.md:213-218`) |
| coordinator tools (owed by ANA-34 §10.11) | missing | MOD-136 (`HANDOFF.md:431-438`) |
| `requirement_cite` | deferred by MOD-11 OQ-7, never re-homed | none (`docs/decisions/mod/mod-11.md:159`) |

## 3. What other harnesses expose

### 3.1 Coding-agent harnesses

Plain file IO, shell and web tools are left out; every harness has them.

| Harness | Agent-facing tools beyond file IO and shell | Gating | Source |
|---|---|---|---|
| Claude Code | `AskUserQuestion` (multiple choice plus "Other", may time out and continue); `EnterPlanMode`/`ExitPlanMode` (exit asks approval); `TaskCreate`/`TaskUpdate`/`TaskList`; `Agent`, `SendMessage`, `ListAgents`, `TaskStop`, `SubagentHandback`; `Workflow`; `Monitor`; `PushNotification`; `ReportFindings`; `CronCreate`, `ScheduleWakeup`; `ToolSearch`, `ListMcpResources` | plan exit, `Workflow`, `Monitor`, web need permission; questions and todos do not | https://code.claude.com/docs/en/tools-reference |
| OpenAI Codex CLI | `request_user_input` (1-3 questions, 2-3 options, recommended first); `update_plan`; `get_context_remaining`; `new_context`; `send_message_to_user_async`; `request_permissions`; `wait_for_environment`; `current_time`, `sleep`; multi-agent `spawn_agent`, `send_input`, `wait_agent`, `list_agents`, `close_agent`, `interrupt_agent`; `/goal` with `create_goal`/`update_goal` (reason required) | per approval mode | https://github.com/openai/codex/tree/main/codex-rs/core/src/tools/handlers ; https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex |
| Gemini CLI | `ask_user`; `write_todos`; experimental `tracker_*` with dependencies; `enter_plan_mode`/`exit_plan_mode`; `complete_task` (subagent returns to parent); `save_memory` (writes straight into future context); `get_internal_docs` | mutators confirm | https://geminicli.com/docs/reference/tools/ ; https://google-gemini.github.io/gemini-cli/docs/tools/memory.html |
| OpenHands SDK | `finish`, `task_tracker`, `think`, `delegate`, `task`, `child_conversation` (own worktree), `ask_oracle`, `planning_file_editor` | `readOnlyHint` annotations; AlwaysConfirm / NeverConfirm / ConfirmRisky | https://github.com/OpenHands/software-agent-sdk/tree/main/openhands-tools/openhands/tools ; https://docs.openhands.dev/sdk/arch/tool-system |
| Cline | `ask_followup_question`, `attempt_completion` (every turn calls one or a tool), `plan_mode_respond`, `new_task` (fresh task from a brief), Focus Chain todo list re-injected every 6 messages | plan mode cannot switch itself to act | https://docs.cline.bot/exploring-clines-tools/cline-tools-guide ; https://docs.cline.bot/features/focus-chain |
| Roo Code | `ask_followup_question`, `attempt_completion`, `switch_mode`, `new_task`, `update_todo_list`, `codebase_search`, `read_command_output` (fetches truncated output) | per mode | https://roocodeinc.github.io/Roo-Code/advanced-usage/available-tools/tool-use-overview |
| goose | Todo, Chat Recall (search past sessions), Summon/delegate, Memory (injected into instructions), Extension Manager | subagents cannot change extensions | https://goose-docs.ai/docs/mcp/chatrecall-mcp/ ; https://goose-docs.ai/docs/mcp/memory-mcp/ |
| Amp | `Task`, `todo_write`/`todo_read`, `oracle`, Librarian, `find_thread`/`read_thread` (search and read past threads), `handoff` (new thread from an editable draft brief) | no approval by default | https://ampcode.com/docs/tools ; https://ampcode.com/news/find-threads ; https://ampcode.com/news/handoff |
| Cursor | ask questions while working; Plan Mode (the user edits the plan); foreground/background subagents | not on the fetched pages | https://cursor.com/docs/agent/tools ; https://cursor.com/docs/agent/plan-mode |
| Devin | MCP: `ask_question`, `devin_session_create`/`search`/`interact`/`events`/`gather`, `devin_knowledge_manage`; Knowledge *suggestions* a person accepts before saving; managed child Devins | suggestions need a person | https://docs.devin.ai/work-with-devin/devin-mcp ; https://docs.devinenterprise.com/product-guides/knowledge |
| Factory Droid | `TodoWrite`, `Task` with `TaskOutput`/`TaskStop`, `AskUser` (disabled in subagents), `ExitSpecMode` (approval) | subagents cannot spawn or ask | https://docs.factory.ai/cli/configuration/custom-droids ; https://docs.factory.ai/cli/user-guides/specification-mode |
| GitHub Copilot coding agent | works on a draft PR; `report_progress` commits and pushes (community-reported) | MCP tools run without approval; GitHub advises read-only allowlists | https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/configure-mcp-servers ; https://github.com/orgs/community/discussions/167906 |
| Jules | "Approve plan", which auto-approves if the user does not answer | none public | https://blog.google/innovation-and-ai/models-and-research/google-labs/jules/ |
| Zed | `diagnostics`, `spawn_agent`, `skill` | allow / deny / confirm per tool | https://zed.dev/docs/ai/tools |
| Aider | none: `/run`, `/test`, `/architect` are typed by the human | n/a | https://aider.chat/docs/usage/commands.html |

### 3.2 Trackers, CI, memory servers and agent frameworks

| System | What it exposes that matters here | Source |
|---|---|---|
| GitHub MCP | toolsets switchable, `--read-only`; `issue_read` (sub-issues, parent), `pull_request_read` (diff, reviews, check runs); `get_job_logs` with `failed_only` and `tail_lines`; lockdown mode filters content by author; `assign_copilot_to_issue_with_intent` marks a model's choice `is_suggestion` | https://github.com/github/github-mcp-server |
| Linear (MCP and Agent API) | read-only endpoint; agent sessions with typed activities (thought, action, elicitation, response, error); session state derived from the last activity; a plan checklist always replaced whole | https://linear.app/docs/mcp ; https://linear.app/developers/agent-interaction |
| Atlassian Rovo, Plane | permission groups by intent (read, write, delete); one resource tool per entity with an `action` parameter | https://developer.atlassian.com/cloud/rovo-mcp/guides/supported-tools/ ; https://developers.plane.so/dev-tools/mcp-server |
| Taskmaster | tiers of 7, 15 or 36 tools; `next_task`, `add_dependency`, `validate_dependencies`; all 36 tools cost about 21,000 tokens | https://docs.task-master.dev/capabilities/mcp |
| Buildkite MCP | `search_logs` (regex with context), `tail_logs`, `read_logs`; `get_failed_executions`; `compare_builds` capped at 3 failures x 20 entries x 8 KiB | https://buildkite.com/docs/apis/mcp-server/tools |
| Sentry, Sourcegraph | tool groups ("skills") selectable per connection; keyword and natural-language search, `commit_search`, `diff_search` | https://github.com/getsentry/sentry-mcp ; https://sourcegraph.com/docs/api/mcp |
| MCP memory server, mem0, Graphiti | direct memory writes (`create_entities`, `add_memory`), scoped by user/agent/run or `group_id` | https://github.com/modelcontextprotocol/servers/tree/main/src/memory ; https://docs.mem0.ai/platform/mem0-mcp ; https://github.com/getzep/graphiti/blob/main/mcp_server/README.md |
| OpenAI Agents SDK | handoffs as `transfer_to_<agent>` with a validated `input_type`; `needs_approval` pauses with serialisable, replay-protected state | https://openai.github.io/openai-agents-python/handoffs/ ; https://openai.github.io/openai-agents-python/human_in_the_loop/ |
| Claude Agent SDK, agent teams | subagents with fresh context, restricted tools, only the final report returned, caps on depth (3), concurrency (20) and budget; teams share a task list with `blockedBy`, and a teammate cannot supply consent | https://code.claude.com/docs/en/agent-sdk/subagents ; https://code.claude.com/docs/en/agent-teams |
| LangGraph / LangChain HITL | `interrupt()` with `response_schema`, resume re-runs the node; decisions approve / edit / reject / respond, and "respond" must not deny side-effecting tools | https://docs.langchain.com/oss/python/langgraph/interrupts ; https://docs.langchain.com/oss/python/langchain/human-in-the-loop |
| CrewAI, AutoGen | delegate / ask coworker tools, only the manager delegates; `HandoffTermination` to "user", agents confusing "ask" with "done" | https://docs.crewai.com/en/concepts/collaboration ; https://github.com/microsoft/autogen/issues/4127 |
| MCP elicitation | flat primitive schema, accept / decline / cancel, MUST NOT ask for secrets | https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation |
| OpenRig (v0.6.3) | 18 operator tools; `rig_send` and chatroom (agent messaging), `rig_capture`, `rig_agent_validate`/`rig_rig_validate` | https://raw.githubusercontent.com/mvschwarz/openrig/v0.6.3/packages/cli/src/mcp-server.ts |

### 3.3 Where the field converges, and where htui stands

| Pattern | Who ships it | htui today | Where it lands in this doc |
|---|---|---|---|
| A blocking structured question (1-3 questions, options, free text) | Claude, Codex, Gemini, Cline, Roo, Factory, Cursor; MCP elicitation | none | MOD-75, with design input (§7.2) |
| An agent-owned todo or plan checklist | about ten harnesses | none | rejected for htui: built in, MOD-110 maps it (§10) |
| Plan mode whose exit asks approval | Claude, Gemini, Factory, Cline, Cursor, Jules | none | MOD-140's `plan_submit` under `runtime_changes = ask` |
| Explicit completion / handback | Cline, Roo, OpenHands, Gemini, Claude | `document_write` plus `missing_output` | rejected as a separate tool (§10, `step_result`) |
| Spawn plus wait / list / stop | Claude, Codex, OpenHands, Devin, Factory, Zed | none | MOD-27 consult, blocking; no lifecycle family (§7.2) |
| New context from a written brief | Codex, Cline, Amp | none | rejected: each step is already a fresh session (§10) |
| Memory written by the model | Gemini, goose, mem0 | none | only Devin's accepted-suggestion form; ANA-38 |
| Search over past sessions | Amp, goose, Devin | none | MOD-112 (4), plus a same-item transcript tail (§8 Q11) |
| CI-shaped log reads and structured failures | Buildkite, GitHub | a 64 KiB tail | **MOD-152** (P2), **MOD-153** (P3) |
| Tool groups and read-only switches | GitHub, Sentry, Atlassian, Taskmaster | per-session predicates | kept: the advertised predicate per role (§7.4) |
| Model choices marked as suggestions, content marked by author | GitHub intent flag, lockdown mode; Devin | none | author marks on every read (§7.4), pending links (**MOD-155** (P5)) |

## 4. Transcript evidence

### 4.1 The corpus and the aggregate numbers

2,833 JSONL transcripts under the project's Claude Code store: 267 top-level sessions, 630 direct
subagents and 1,936 workflow agents, 2026-09-18 to 2026-10-18 (`/tmp/ana36/transcripts-aggregate/agg.py`,
`agg2.py`, `agg3.py`; output `out.json`, `out2.json`, `out3.json`). The session running ANA-36
(`a05d1b2e`) is excluded from the episode passes.

| Measure | Number | Key |
|---|---|---|
| Tool calls | 105,830; Bash 74,411 (70.3%); all Gortex tools 16,470 | `out.json tool_counts_all` |
| htui MCP calls | 0 | `out.json mcp_servers` |
| `cargo test` runs | 8,766; 91% piped through `tail`/`head`/`grep`; 2,296 cargo runs redirected to a `/tmp` log | `out2.json cargo` |
| Failed `cargo test` results, and re-runs | 2,517 failed (28.7%); 978 re-runs after a failure with no edit in between | `out2.json cargo.failed:test`, `rerun_after_fail_no_edit_between:test` |
| Sleep polling | 1,090 Bash calls with `sleep N`, 922 of them sleep-then-check; 169 Reads of background `.output` files | `out2.json poll` |
| Explicit sleep time | about 25.3 h in the newer half (169 transcripts); 41,722 s of sleep-first calls in 17 older sessions | `/tmp/ana36/transcripts-episodes-newer/episodes.py`; older `quant.py` |
| Planning-doc reads through shell text tools | 8,380 Bash calls (sed, grep, wc); 1,361 greps for R-/MOD-/ANA-/D- IDs; 2,047 of 2,833 sessions read planning docs | `out2.json plandoc_read`, `id_lookup` |
| Planning-doc writes | 2,337, 992 of them through `sed -i`, redirects or python; 58 runs of 5+ consecutive Edits on one doc | `out2.json plandoc_write`, `loop_kind` |
| Questions to the user | 191 AskUserQuestion calls (403 questions) in 71 of 267 top sessions; 136 of 495 top-level turn ends end in `?` | `out.json ask_user_question_calls`; `out2.json turn_end_q` |
| Question waits | older half: median 4.4 min, 30 of 95 over 10 min, 10 over 60 min; newer half: median 165 s, p90 about 56 min, about 75 h in total | older `quant.py`; newer `episodes.py` |
| Todo tools | 0 calls to `TodoWrite` or `Task*` | `out.json tool_counts_all` |
| Subagents | 633 `Agent` spawns, 289 `Workflow` runs; 662 of 833 newer subagents run git log/diff/show/status in their first 8 calls | `out.json`; newer `start.py` |
| Hook denials (dev setup, §1) | 4,203; 355 on greps over cargo output, 307 cargo test calls blocked outright | `out2.json hook_gortex_shape`, `hook_gortex_on_cargo` |

### 4.2 The strongest episodes

Line numbers are JSONL line indices in the session file; paths are relative to the project's transcript
directory. Every row below was re-opened for this doc.

| Session | Lines | What happened | What it argues for |
|---|---|---|---|
| `d901debf-e2b6-4a7b-9d0a-fd9c4711a041` | L96, L169-L520 | The parent waited on Workflow implementers with `sleep 240`…`sleep 595; git log --oneline -5`, 42 polls, about 6.6 h | a wait with a deadline (**MOD-152** (P2)'s `command_wait`; MOD-27's consult answer is a tool result) |
| `15230992-43d0-4411-b199-abbae1d61fcd/subagents/workflows/wf_37948082-f02/agent-a5fd239f324e9f274.jsonl` | L310-L311, L359-L365 | `sleep 600; cat …output` hit the 10-minute Bash timeout (exit 143); a later `sleep 590` waited on a clippy gate whose output says it finished in 1m 18s | **MOD-152** (P2) |
| `1cda6355-5bca-49bd-a158-047f3e0fdc85` | L209-L213 | `cargo test --workspace … ; grep -E '^test result\|FAILED\|panicked\|SIGABRT'` was blocked, and the whole suite was restarted without the grep | **MOD-152** (P2)'s stored log, **MOD-153** (P3) (the block itself is the dev hook) |
| `d478a7a8-b1c1-48be-b214-5535e253f10d` | L1160-L1266 | `cargo test -p htui --test skills` re-run about 6 times to see one failure at a time | **MOD-152** (P2), **MOD-153** (P3) |
| `4e526f51-a3be-4706-85df-dd1fd8c083e3` | L571-L821 | the workspace suite ran at least 4 times under `timeout 5400`, above `command_run`'s 1800 s cap, to read totals | **MOD-153** (P3)'s per-preset timeout |
| `c6da34db-a6cc-452d-83f0-d6636d98965d` | L735-L736 | a close-out question asked at 20:12 was answered at 08:51 the next day (759 min), the session held open | MOD-75 (a park that can release the session) |
| `7878cab7-9459-4f24-8104-5187bcc20bd0` | L458-L459 | "Which [review findings] do you want fixed in M1" waited from 17:19 to 00:13 | MOD-75 |
| `167509c2-9ac6-4a5f-ad64-895049b68867`, `d48064b5-9310-4c64-874a-204546d30f91` | L751; L364-L400 | two sessions drove MOD-32 in one worktree; each "corrected" the other's record of the maintainer's CONFIRM time in prose | MOD-75's stored answer and timestamp |
| `0cdc82e8-9877-47f0-835e-3ad703fdb3a2` | L13, L35, L182, L300 | the maintainer asked "what are the next items I can run in parallel" six times; each answer was HANDOFF greps and branch probes | MOD-112's `item_query`; this is coordinator work (MOD-136) |
| `dba80770-f39c-4527-9288-abfdf1329292` | L34-L102 | MOD-4's milestone state rebuilt from `git log --all --grep` because the PRD row still said `pending` | MOD-112's `item_read` with runs and steps |
| `973a13d4-1784-411f-87e7-8fe073f3539f` | L221-L236 | placeholder item IDs remapped to minted IDs with a hand-written python script | `item_create` drafts (MOD-112 (3)); store-minted keys already exist (`R-ENT-7`) |
| `a5db7a19-7fab-485b-81e5-19ca6a43f14f` | L497-L511 | an agent amended `R-AGT-8` in `REQUIREMENTS.md` with two direct Edits | `requirement_propose` as a draft (MOD-118, MOD-136) |
| `64060544-c952-42c1-9cc3-10bebb4cd4fe` | L701-L704 | a parent scraped a Workflow `journal.jsonl` with ad-hoc python, the first attempt printing nothing | evidence about Claude's Workflow harness, not htui (skeptic) |
| `7bc58ece-43ad-409d-927b-89f9ffa8b422` | L776-L1004 | the DSN for the Postgres gate hit the auto-mode classifier, then a docker-inspect script, a classifier outage, a placeholder host and a stray `.sqlx` | named presets (**MOD-153** (P3)); the credential part is met by `R-SEC-2` (skeptic) |
| `200cbe8d-01a4-4023-be7b-508f23c692fb` | L76 | `cat .claude/skills/handoff-run/references/routing.md`: a skill reference read on demand | `skill_resource_read` (MOD-113 (3)), gated on `R-ID-5` (§8 Q10) |

### 4.3 What the transcripts do not show

No interactive TUI or PTY driving worth a tool (33 pexpect/pty calls in 4 session trees), no images (0
pasted, 0 image Reads), no use of todo tools, and no demand for a second-opinion model beyond what
`Agent` spawns already serve (`/tmp/ana36/critic/pty.py`, `img.py`).

## 5. The candidate catalogue

36 candidates: the catalogue's 33 plus the critic's three. Verdict column: **holds** (skeptic found no
refutation), **holds, corrected** (corrections applied in the row), or **refuted** (the reason is in
§10). "Value/cost" is the skeptic's corrected value. Transcript ids are 8-character session prefixes
(§4.2 has the full ids). Code paths are abbreviated as in §2.

| # | Name | What | Problem replaced (evidence) | Kind | `R-MCP-1` scope | `R-ID-6` line | Owner | Value / cost | Skeptic verdict |
|---|---|---|---|---|---|---|---|---|---|
| 1 | `item_read` / `item_query` | one item: fields, notes marked by author, links both ways with each target's status, documents by kind, runs and steps; a list variant by status, prefix, tag or link | search hits cannot be opened (`search.rs:85-107`); HANDOFF greps (496 calls in 103 older sessions); `0cdc82e8`, `dba80770` | read | active workspace; out-of-workspace upstream items as one-line stubs (`R-PRM-2`) | deterministic read; author marks; other runs' drafts never returned | MOD-112 (1) | high for chats and coordinators, medium for phase steps / small (lift `ReadStore::items`, `links`) | holds, corrected: phase prompts already carry the item (`R-PRM-1`); "proposed" links need **MOD-155** (P5) first |
| 2 | `document_read` | a document by item and kind or id; outline first, then one section by heading or anchor | `R-PRM-3` trims inputs with no way back; offset paging of design docs (`d901debf` subagent L21-L45) | read | active workspace | deterministic | MOD-112 (1) | high where inputs are trimmed / small | holds: the read exists (`worker.rs:145-158`); trim is the real driver |
| 3 | `requirement_read` | requirements by id or prefix with text, state, version and citing items | per-id `git grep` loops (`d41b8799` subagent ~L46); no requirement read on `WorkerStore` | read | project requirements until ANA-35's `W-` keys | deterministic | MOD-112 (1) | high / small | holds, corrected: citation kinds are five (`requirement.rs:33-43`, `R-ENT-15`) |
| 4 | `search_concepts` widening | workspace scope, notes and scrubbed transcripts; every hit openable | project-only search (`tools/search.rs:101-108`) | read | workspace index | deterministic; fails open (`R-NF-2`) | MOD-112 (2), (4); ANA-37 | medium / medium | holds, corrected: the worker offers search when its keyring holds the URL (`worker_cmd.rs:102-105`) |
| 5 | `item_create` (draft) | propose an item; a person accepts it into a real item | hand-minting and placeholder remaps (`973a13d4` L221-L236) | draft | draft owned by the run; item exists only on acceptance | a draft no reader consumes before acceptance | MOD-112 (3); store MOD-135 | high / medium | holds, corrected: minting pain is markdown's (`R-ENT-7`); use a stable draft id, not a rewrite of document bodies |
| 6 | `ask_user` | 1-3 questions with options and a recommendation; parks the step; answer and time stored | 191 calls in 71 sessions; waits up to 759 min (`c6da34db`); a false CONFIRM time in prose (`167509c2`) | action | the owning step | question display-only; the answer is a person's; never auto-resolves | MOD-75 | high / medium | holds: add the part-versus-run park question (`origin/hr/ANA-33:HANDOFF.md:476-478`) |
| 7 | `plan_bounds` / `plan_submit` / `plan_status` | the planner's bounds, a fenced plan proposal, plan state | `R-MCP-2`'s planner tools | draft | the run's own plan | item work; gated nodes never bypassed; no auto-approve | MOD-140 (`origin/hr/ANA-33`) | high / large | holds: nothing for ANA-36 to add (`origin/hr/ANA-33:docs/ANA-33.md:573-618`) |
| 8 | `spawn_subagent` (consult) + lifecycle | a read-only consult child; wait / list / message / cancel calls | 4.5 KB environment preambles (`167509c2` subagent L1); journal scraping | action | children of this step | item work | MOD-27 | high for consult, low for the lifecycle calls / large | holds, corrected: consult answers as a blocking tool result, so no lifecycle family; cancel is MCP cancellation (`protocol.rs:336-347`) |
| 9 | coordinator tools | read runs and waiting rows below; drafts; resolution and re-confirm proposals; enqueue or link through a permission card | ANA-34 §7.1 | draft | item-less scope | agent drafts, code disposes | MOD-136 | high / large | holds |
| 10 | close-out proposal | resolution, follow-ups, requirement changes in one proposal | close-out python surgery (`595a3f27` L254-L330) | draft | the run's item | draft | — | low | **refuted**: a composite of #13, #5 and #15 feeding MOD-135 |
| 11 | memory / knowledge suggestion | propose a durable fact as a draft | 96 re-reads of memory files to relay gotchas (newer `mem.py`) | draft | run-owned draft, project or workspace entry | a draft a person accepts before any prompt reads it | ANA-38 | medium at most / medium | holds, corrected: phase-bound skills already deliver standing gotchas |
| 12 | `item_link` as a draft | scheduling kinds pending until accepted; `R-ENT-9`'s kinds; workspace targets | a live `blocked_by` changes auto-mode picks (`pg/write.rs:7137-7146`, `pg/read.rs:2097-2100`) | draft | `from` the run's item; `to` in the workspace | pending for `blocked_by`, `supersedes`, `duplicates` (and `parent`, §8 Q2) | unowned → **MOD-155** (P5); kinds ANA-35 | high / medium | holds, corrected: reverses MOD-11 OQ-4 (`mod-11.md:156`), needs the maintainer |
| 13 | `item_status` as a typed request | a typed request row (open/blocked/closed + resolution + reason) on waiting-on-you and the close-out checklist | requests surface only as the latest note (`pg/read.rs:2357-2362`); eight statuses vs `R-ENT-8`'s three | draft | the run's item | a request a person accepts; never moves status (I-4) | unowned → **MOD-154** (P4) | medium / small | holds, corrected: "unseen" overstated; absorbs #10 |
| 14 | `requirement_cite` | propose an `addresses`, `reserves` or `verifies` citation; withdraw only the run's own | deferred by MOD-11 OQ-7 (`mod-11.md:159`); no fenced path (`traits.rs:1775-1789`) | draft | from the run's item; target readable under `R-MCP-1` | pending until accepted, because coverage reads citations | unowned → **MOD-155** (P5) | medium / small | holds, corrected: kinds; never overwrite a person's proposer (`cite` does on revive) |
| 15 | `requirement_propose` | a step agent drafts a requirement change or split | direct Edits of `R-AGT-8` (`a5db7a19` L497-L511) | draft | run-owned draft | a draft; `amend_requirement` stays forbidden | MOD-118 (step agents), MOD-136 (coordinator); store MOD-135 | medium / medium | holds, corrected: owned, name the tool there |
| 16 | `draft_update` | the talk-it-through agent edits the live draft in the `R-TUI-17` modal | `R-TUI-17` names no tool | draft | the modal's unsaved draft | a draft a person accepts | MOD-119 (items), MOD-118 (requirements) | medium / small | holds, corrected: owned, not unowned |
| 17 | htui agent tools | read and change settings, Toolbox, agent rows, targets, catalogue | `R-AGT-11` names no tool surface | action | the global htui agent chat | additions deterministic; secrets and targets person-gated | MOD-121 | medium / large | holds, corrected: secrets through an entry form, never as an argument (`R-SEC-2`) |
| 18 | `command_start` / `command_wait` / `command_cancel` | an async split of `command_run` over the same queue rows | sleep polling (about 25 h newer half; `15230992` subagent L310-L365); 30-minute blocking calls and client timeouts (`docs/htui-mcp.md:323-327`) | action | the step's own queue rows | deterministic | unowned → **MOD-152** (P2) | high where `command_run` is exposed (fan-out by default) / medium | holds, corrected: one call *in flight* per session, not one per session |
| 19 | `command_output_read` | tail, range or regex with context over a stored scrubbed log of a run's command | 978 no-edit re-runs; `/tmp` log greps | read | command rows of the caller's run | deterministic; the stored log is scrubbed fail-closed | unowned → **MOD-152** (P2) | medium / medium | holds: only the tail is stored today (`command.rs:355-377`), so a log artifact is new |
| 20 | structured test results | per-target counts, failing tests with excerpts, crash signals, pending snapshots; `rerun_failed`; the run's verify result read-only | 8,766 test runs, 91% filtered by hand; suites re-run 4-6 times (`d478a7a8`, `4e526f51`) | action | the run's worktree and queue | deterministic parsing | unowned → **MOD-153** (P3) | medium / medium | holds, corrected: pluggable parsers keyed on recognised runners (nextest, JUnit), fallback to #19 |
| 21 | `gate_run` | named project gates with secrets injected into the child only | DSN classifier walls (`7bc58ece`) | action | the run's worktree | — | — | low as a tool | **refuted**: `R-SEC-2` already resolves secrets into the agent env; remainder folded into **MOD-153** (P3) as presets |
| 22 | `command_run` in `extra_dirs` | a `repo` argument resolving to the session's extra repos | `directory()` joins only under `scope.cwd` (`command.rs:330-353`) | action | repos the engine passed to the session | deterministic | unowned → **MOD-152** (P2) | medium / small | holds: `ToolScope` needs `extra_dirs` |
| 23 | `session_info` | the session's own scope, offered tools and why, attempt, base commit | no `instructions`; the prompt names two tools | read | the calling session | deterministic | unowned → folded into **MOD-151** (P1) | medium / small | holds, corrected: the cheap part is `initialize.instructions`; attempt and base commit belong in the prompt |
| 24 | `run_history_read` | an item's runs, steps, outcomes, commits; a scrubbed tail of a named step of the same item | MOD-4 state from git log (`dba80770`) | read | beyond the run; transcript tails only for the same item | deterministic; past model text marked as a step's output | MOD-112 (1) for the listing; the tail is unowned (§8 Q11) | medium / small | holds, corrected: fold the listing into #1 |
| 25 | `run_steps_read` | the run's step graph and sibling results | journal scraping (`64060544`) | read | the caller's run | deterministic | — | low | **refuted**: prior steps reach later ones as inputs; planner and integrate views are MOD-140's `plan_status` |
| 26 | `sibling_runs` | other live runs, files touched, leases | `ps aux \| grep rustc` contention checks (`96d6ff3d`) | read | beyond the run | deterministic | — | low | **refuted**: the queue's class limits do this; coordinator reads are MOD-136's |
| 27 | `step_result` | a typed handback with an outcome claim and findings | StructuredOutput failures in Workflow subagents (`cf9ef78a`) | write | the caller's step | an outcome claim nothing may read | — | low | **refuted**: `document_write` is the work product; at most a per-phase schema on it |
| 28 | `document_patch` | anchored section edits to the step's own output document, one version per call | 58 runs of 5+ Edits on one doc; output tokens on large bodies | write | the step's output document | item work product | unowned → **MOD-156** (P6) | low to medium / small | holds, corrected: anchors can still miss; the saving is output tokens |
| 29 | `notify_user` | a non-blocking message to the person | none in transcripts | write | the owning step | display-only | — | low | **refuted**: `note_add` with a notify flag MOD-114 rings on |
| 30 | `resource_lease` | lease a named shared resource with a TTL | `flock /tmp/modNN-pg.lock` wrappers (`be428bb6`) | action | the run | deterministic | — | low | **refuted**: a project-defined command class with limit 1 (MOD-109) |
| 31 | `box_profile` health | live disk, processes, Postgres reachability | 1,517 `df -h` calls | read | the session's box | deterministic | — | low or none | **refuted**: the shell has these; it would break the prompt-and-tool agreement |
| 32 | `handoff_draft` | a continuation brief a person accepts | one session-limit event (`4ade4808` subagent) | draft | the run | draft | — | low | **refuted**: each step is a fresh session; continuation is MOD-140 `successors` or MOD-111 (3) |
| 33 | `sibling_message` | a message to another live step of the run | none | action | live steps of the run | not an `R-ID-6` breach | — | low / large | **refuted**: breaks `R-PRM-1`'s self-contained prompt; needs a live channel; injection vector |
| 34 | `skill_resource_read` | list and read a bound skill's resources, outline first | 362 tool-use lines reading `handoff-run/references` (`200cbe8d` L76; critic `skill.py`) | read | skills bound to this step only | deterministic, person-authored content | MOD-113 (3), gated on `R-ID-5` | low to medium / small | holds, corrected: the demand is this repo's workflow skill; conflicts with `R-ID-5` |
| 35 | `attachment_read` / `attachment_add` | read an item's or document's attachment; attach a log or file from the run | `R-ENT-12` attachments; no transcript demand | read / write | read: workspace; add: the run's item or document | add is a work product; images cannot be text-scrubbed | unowned → **MOD-157** (P7); storage MOD-114 (3), schema ANA-35 | low / medium | holds: sequence after MOD-114 (3) |
| 36 | `definition_validate` | validate a draft persona, graph, template or skill without saving | none | read | workspace lookups | deterministic | — | low | **refuted**: validate inside the MOD-121 and `draft_update` write tools, as `plan_submit` does |

## 6. Options and trade-offs

**6.1 How agents learn the tools.** A, a `session_info` tool the agent calls; B, fill the MCP
`initialize.instructions` field and give the prompt a short section naming every advertised tool and
when to use it; C, both. A spends a turn on what the prompt could say (`R-ID-5`: agents never spend turns
reading instructions) and restates `R-PRM-1`'s prompt. B is cheap and reaches every agent at session
start. Its one cost: every prompt byte is a digest input, so rewording `COMMAND_QUEUE_TEXT` invalidates
golden prompts and stored digests (`defaults.rs:21-25`). **B**, with the `R-PRM-3` trim record named in the
prompt so the agent knows what `document_read` should fetch.

**6.2 Long commands.** A, keep `command_run` blocking and tell people to raise the client's tool timeout
(today's answer, `docs/htui-mcp.md:323-327`); B, add a background flag to `command_run`; C, a
`command_start` / `command_wait` / `command_cancel` family over the same rows, keeping `command_run` as the
blocking form. A leaves the sleep polling where it is, and while `command_run` is exposed the harness
shell refuses `cargo test` and friends, so the agent cannot background them itself. B overloads one tool's
answer shape. **C**: each call has one answer shape; `command_wait` takes an absolute deadline and an
optional pattern, so no call outlives the client's timeout and no agent sleeps blind.

**6.3 Reading output.** A, raise the stored tail; B, store a scrubbed, capped log artifact per command
row and read it by tail, range or regex; C, parse everything into structure. A still truncates. C alone
fails on runners htui does not parse (htui serves any project type; R-MCP-4's prefixes include cmake,
ctest, dotnet, npm, go). **B first (MOD-152 (P2)), C on top as pluggable parsers with B as the fallback (MOD-153 (P3)).**

**6.4 Named gates.** A, a `gate_run` tool; B, a `preset` argument on `command_run` / `command_start` that
resolves to a project-configured command line, working directory and timeout. The skeptic refuted A: the
secret-isolation motive is already met by `R-SEC-2`, and "only in the child" does not isolate anything
from agent-written test code. **B**, which also lifts the 1800 s cap where a project configures a longer
gate.

**6.5 Agent links.** A, keep every agent link live (MOD-11 OQ-4); B, every agent link pending until a
person accepts it; C, pending only for the kinds a scheduler, status or subgraph reads (`blocked_by`,
`supersedes`, `duplicates`, `parent`), live for display kinds (`relates`, `origin`, `implements`). A
contradicts `R-ID-6` ("a suggested link … written only once a person accepts", `REQUIREMENTS.md:115-118`)
in the one place it bites: auto mode's picks. B adds a person's click to every `relates`. **C**, with the
acceptance on the item's links view and as a waiting row; ANA-34 already routes a coordinator's
`blocked_by` through a permission card (`docs/ANA-34.md:536-538`).

**6.6 Status requests.** A, today's note; B, a typed request row a person accepts or rejects, shown on
waiting-on-you and on MOD-135's checklist. A is visible only until a newer note arrives. **B**, which also
absorbs the refuted close-out proposal (#10).

**6.7 How many tools a session sees.** Every advertised tool costs prompt tokens (Taskmaster: about
21,000 for 36 tools). htui already advertises per session by predicate; the new tools keep that: the
command family only where `command_run` is exposed, `document_patch` only with `document_write`, the
htui-agent and modal tools only in those chats, consult children without `ask_user` (Factory and Codex
deny it too). No toolset switches beyond the predicates.

## 7. Verdict and phasing

### 7.1 Verdict

**htui needs fewer new tools than the field ships, because its prompt and store already do much of what
other harnesses hand to the model.** Each step is a fresh session with a self-contained prompt
(`R-PRM-1`), so handoff briefs, sibling messages and step-graph reads add little. Status, close-out and
scheduling stay deterministic (`R-ID-6`, MOD-11 I-4), so completion and progress tools would carry model
text nobody may read. What agents lack is: **telling them about the tools they have; long commands
that do not block or force re-runs; a few typed requests and proposals that stay pending until a
person accepts them; and the reads MOD-112 already owns.** The rules every new tool follows are in §7.4.

### 7.2 Covered by an existing item, with ANA-36's design input

These are not re-proposed. Each owner gets the input on the right when ANA-36 concludes.

| Owner | Tools (§5 #) | Design input ANA-36 adds |
|---|---|---|
| MOD-112 (`HANDOFF.md:689-696`) | #1, #2, #3, #4, #5, listing half of #24 | `item_read` returns links in both directions with each target's status, notes marked by author (`Note.via_step_id`), documents by kind, and the item's runs and steps with outcomes and commits; `item_query` wraps `ReadStore::items`; out-of-workspace upstream items return as one-line stubs (`R-PRM-2`); `document_read` is outline-first and section-addressed, never by line offset, byte-capped with a cursor; `requirement_read` returns all five citation kinds; `ToolScope` gains a workspace (coordinate with MOD-136's item-less scope); `item_create` drafts carry a stable draft id that documents reference, and htui resolves it on display instead of rewriting bodies. Same-item step transcript tail: §8 Q11. |
| MOD-75 (`HANDOFF.md:491-501`) | #6 | the converged shape: 1-3 questions, each with an id, a short header, 2-3 options with the recommended one first, and free text; accept / decline / cancel outcomes; htui stores the answer and its time against the step, and later records cite that row, not model prose; no question tool in consult children; never auto-resolves on timeout; the part-versus-run park question from `origin/hr/ANA-33:HANDOFF.md:476-478`. Builds the tool-to-engine channel (§8 Q8). |
| MOD-140 (`origin/hr/ANA-33:HANDOFF.md:875-883`) | #7 | none beyond cross-reference; ANA-33 §7.6 hands the wider survey to ANA-36 and MOD-112 (`origin/hr/ANA-33:docs/ANA-33.md:616-618`). Note from the critic: changes a part makes outside its owned paths are "shown to integrate and review" (`:604-606`), but `plan_status` is advertised to planner and integrate only (`:584`); the review role needs the facts in its prompt or `plan_status`. |
| MOD-27 (`origin/hr/ANA-33:HANDOFF.md:213-218`) | #8 | the consult answer is a blocking tool result, so no wait / list / message tools; cancel is MCP request cancellation (`protocol.rs:336-347`); the residual issue is the client timeout on a long consult: if consult goes async, reuse **MOD-152** (P2)'s start/wait shape rather than a second lifecycle family; reuse MOD-75's tool-to-engine channel; Claude SDK-style caps on depth, concurrency and budget against `max_agents_per_run`. A second-opinion "oracle" is a consult, not its own tool. |
| MOD-136 (`HANDOFF.md:431-438`) | #9 | share the read code with MOD-112's `item_read` and run listing; coordinator link requests go through **MOD-155** (P5)'s pending state, not a live write. |
| MOD-135 (`HANDOFF.md:424-430`) | store for #5, #15; consumer of #13 | **MOD-154** (P4)'s typed status request is the checklist's "proposed resolution" input; follow-ups come from `item_create` drafts. |
| MOD-118, MOD-119 (`HANDOFF.md:765-782`) | #15, #16 | name `requirement_propose` (MOD-118, step agents; amends and withdraws travel here, not through `requirement_cite`) and `draft_update` (MOD-119 items, MOD-118 requirements) as the modal chats' tools; validate inside the write tool and answer typed refusals, as `plan_submit` does (absorbs #36). |
| MOD-121 (`HANDOFF.md:793-800`) | #17 | name the htui agent's tool surface; a secret is entered through a TUI form the tool opens, never passed as an argument (`R-SEC-2`); validation inside the write tools (absorbs #36). |
| ANA-38 (`HANDOFF.md:149-157`) | #11 | only the accepted-suggestion shape (Devin's), never a direct write into future context; part of the demand is met by phase-bound skills. |
| MOD-113 (`HANDOFF.md:697-705`) | #34 | part (3)'s "the prompt and session learn to deliver them" becomes a read tool over the bound set only, outline-first like `document_read`; scripts are written to a scratch directory outside the repo and run through `command_run`. Gated on §8 Q10. |
| MOD-110, MOD-114 | refuted #29 and the todo tools (§10) | MOD-110 maps the harness's own todo and question events into rows; MOD-114 rings on a `note_add` that carries a notify flag. |

### 7.3 Proposed new items

Seven items for the unowned survivors, ordered by dependency. Filed 2026-10-18 as P1 → MOD-151,
P2 → MOD-152, P3 → MOD-153, P4 → MOD-154, P5 → MOD-155, P6 → MOD-156, P7 → MOD-157.

| Item | Title | Scope | Depends on | Requirements |
|---|---|---|---|---|
| **MOD-151** (P1) | Tell agents about htui's tools | `initialize.instructions` naming each advertised tool and when to use it; a prompt section built from the same table (replaces `session_info`, #23); the prompt names the `R-PRM-3` trim record; reword `COMMAND_QUEUE_TEXT` so it no longer says "verification" (the tool refuses `verify`), accepting the digest change; results gain `structuredContent` with an `outputSchema`, typed stable refusals that are not `isError` (ANA-33's `plan_submit` style), and `readOnlyHint` annotations on reads. | none (MOD-11 done); coordinate with MOD-112 so its tools are described the same way | R-MCP-2, R-MCP-4, R-PRM-1, R-PRM-3 |
| **MOD-152** (P2) | Async command queue and stored command logs | `command_start` / `command_wait` (absolute deadline, optional line pattern) / `command_cancel` over the existing `command_run` rows, same fence, scrub and class limits, advertised with `command_run` and kept in `ASKING_TOOLS`; a scrubbed, size-capped log artifact per row (cap chosen by the plan; R-ID-7 fail-closed); `command_output_read` (tail, line range, regex with context, byte cap) over the caller's run's rows; a `repo` argument resolving the session's `extra_dirs` by name (#22), with `ToolScope.extra_dirs` set by the engine. `command_run` stays as the blocking form. | none; MOD-109 for limits in Settings (not blocking) | R-MCP-3, R-MCP-4, R-ID-7, R-SEC-3 |
| **MOD-153** (P3) | Structured test results and named command presets | Pluggable result parsers keyed on recognised runners (nextest and JUnit XML first), giving per-target counts, failing tests with a capped excerpt, crash signals and pending snapshots, with **MOD-152** (P2)'s log read as the fallback; `rerun_failed`; a read-only structured view of the orchestrator's verify result for the run; a `preset` argument resolving to a project-configured command, directory and timeout (above 1800 s where configured), replacing the refuted `gate_run` (#21). | **MOD-152** (P2); MOD-109 for the preset form | R-MCP-3, R-MCP-4 |
| **MOD-154** (P4) | `item_status` as a typed request | A typed status-request row (requested status from `R-ENT-8`'s open/blocked/closed, resolution, reason, step) that a person accepts or rejects; shown as a waiting-on-you row and as MOD-135's proposed resolution; never moves the status (I-4); the enum narrows when ANA-35's migration lands. Absorbs the refuted close-out proposal (#10). | ANA-35 (status enum); MOD-135 (checklist consumer) | R-ENT-8, R-MCP-2, R-ID-6, R-TUI-11 |
| **MOD-155** (P5) | Link and citation proposals that wait for a person | `item_link`: a pending state for the kinds §8 Q2 settles, which `ready_items`, `queue_rows` and subgraph sources ignore until accepted; accept and reject on the item's links view and as a waiting row; `R-ENT-9`'s `parent`, `duplicates`, `implements` and workspace targets once ANA-35 adds them. `requirement_cite`: a new fenced `WorkerStore` method proposing `addresses`, `reserves` or `verifies`, pending until accepted, withdrawing only the run's own, never overwriting a person's citation or proposer. | ANA-35 (kinds, `W-` keys); §8 Q2, Q3; the `R-MCP-2` amendment (§9) | R-ENT-9, R-ENT-15, R-ID-6, R-MCP-1, R-MCP-2 |
| **MOD-156** (P6) | `document_patch` for the step's output document (wanted later) | Anchored edits (replace a section, upsert a table row, append under a heading) to the latest version of the step's own output document in one call, one new version per call, typed refusal on an anchor miss; advertised with `document_write`. | none | R-MCP-2 |
| **MOD-157** (P7) | Attachment tools (wanted later) | `attachment_read` (images as MCP image content, text capped and scrubbed, others as metadata) at workspace read scope; `attachment_add` from the run's worktree or a stored command log to the run's item or output document, under the size cap, text and logs only unless a person confirms (images cannot be text-scrubbed, `R-ID-7`). | MOD-114 (3); ANA-35 (who owns an attachment) | R-ENT-12, R-MCP-1, R-ID-7 |

**Phasing.** **MOD-151** (P1) and **MOD-152** (P2) are not blocked and carry the most value for their
cost; **MOD-153** (P3) follows **MOD-152** (P2). **MOD-154** (P4) and **MOD-155** (P5) wait on ANA-35 and
on §8; **MOD-155** (P5) also waits on the `R-MCP-2` amendment. **MOD-156** (P6) and **MOD-157** (P7) are
filed as wanted later. Effects on existing items, applied at acceptance: MOD-112, MOD-75, MOD-27 (on its
ANA-33 text), MOD-118, MOD-119, MOD-121 and MOD-113 gain the design input of §7.2; MOD-109 gains
**MOD-152** (P2) and **MOD-153** (P3) as consumers of the limit and preset forms; MOD-135 gains
**MOD-154** (P4) as its resolution input.

### 7.4 Rules every tool follows

Condensed from the catalogue's 17 cross-cutting rules, with the skeptic's corrections.

1. **Scope from the token, never from arguments** (MOD-11 I-1, `deny_unknown_fields` kept). Reads may
   widen to the active workspace; writes stay on the owning step. Out-of-workspace items read as stubs.
2. **Propose versus dispose.** Model text written through a tool is an item work product, a display-only
   label, or a draft (MOD-135's store, or a pending row) that no status move, scheduler, prompt section,
   import or search index reads before a person accepts it.
3. **Every read marks its author** (a person, an agent with its step, an accepted draft); a run's
   unaccepted drafts are visible only to that run.
4. **Every write is fenced** through a `WorkerStore` method that takes a `StepFence` (I-3); new writes
   need new fenced methods.
5. **Everything persisted or returned is scrubbed, failing closed** (I-5, `R-ID-7`), including **MOD-152** (P2)'s
   log artifacts. No tool reads a secret (`R-SEC-2`, `CONCEPTS.md:90-91`).
6. **A tool not advertised does not exist** (I-7); exposure is per role and phase.
7. **Typed results**: `structuredContent` with an `outputSchema`; refusals typed, stable and not
   `isError`.
8. **Large reads** are outline-first, section-addressed, cursor-paged and byte-capped.
9. **Long operations** return a handle and a wait with an absolute deadline; progress notifications
   where the client asks; no call blocks past a client's timeout.
10. **Human gates never auto-resolve**; a relayed agent message never carries consent (ANA-27 T2,
    `docs/ANA-27.md:619`).
11. **Approval posture**: reads and drafts pre-approved; side-effecting actions (the command family,
    consult spawn, htui-agent changes) stay in `ASKING_TOOLS` under the `ask` policy.
12. **One tool-to-engine channel** for the tools that need the engine mid-session (MOD-75's park,
    MOD-27's consult), designed next to `permission_prompt`'s prompt bridge.
13. **Search-backed tools fail open** with "search unavailable" until ANA-37 decides (`R-NF-2`).
14. **Naming** keeps the `noun_verb` style: `item_read`, `command_start`, `command_output_read`.
15. **New engine futures are boxed** (I-8, stack headroom).

## 8. Questions for the maintainer

1. **The catalogue verdict (§5, §7.2):** 11 candidates refuted (§10), 15 left with their owners plus the
   design input of §7.2, 10 unowned survivors grouped into **MOD-151** (P1) to **MOD-157** (P7). **Recommended: accept.**
   **Answered 2026-10-18: accepted as recommended.**
2. **Agent links (6.5, MOD-155 (P5)):** an agent's link of a kind a scheduler, status or subgraph reads
   (`blocked_by`, `supersedes`, `duplicates`, `parent`) is pending until a person accepts it; `relates`,
   `origin` and `implements` stay live as display. This reverses MOD-11 OQ-4 for those kinds and reads
   `R-ENT-9`'s "proposed by agents" together with `R-ID-6`. **Recommended: accept.**
   **Answered 2026-10-18: accepted as recommended.**
3. **`requirement_cite` (MOD-155 (P5)):** add it to `R-MCP-2` as a pending citation of kind `addresses`,
   `reserves` or `verifies`; `amends` and `withdraws` mean a text change and travel through
   `requirement_propose` (MOD-118). **Recommended: accept.**
   **Answered 2026-10-18: accepted as recommended.**
4. **Status requests (6.6, MOD-154 (P4)):** `item_status` writes a typed request row a person accepts or rejects,
   shown on waiting-on-you and MOD-135's checklist, and still never moves a status.
   **Recommended: accept.**
   **Answered 2026-10-18: accepted as recommended.**
5. **Long commands (6.2, 6.3, MOD-152 (P2)):** keep `command_run` blocking and add `command_start` /
   `command_wait` / `command_cancel`, a stored scrubbed log per command row, `command_output_read`, and a
   `repo` argument for `extra_dirs`. **Recommended: accept; the log cap is the plan's.**
   **Answered 2026-10-18: accepted as recommended.**
6. **Results and presets (6.4, MOD-153 (P3)):** pluggable parsers with the log read as fallback, a read-only view
   of the run's verify result, and named presets with their own timeout instead of a `gate_run` tool.
   **Recommended: accept, after MOD-152 (P2).**
   **Answered 2026-10-18: accepted as recommended.**
7. **Tool guidance (6.1, MOD-151 (P1)):** fill `initialize.instructions` and a prompt section naming every
   advertised tool, instead of a `session_info` tool; reword `COMMAND_QUEUE_TEXT` (this changes prompt
   digests). **Recommended: accept.**
   **Answered 2026-10-18: accepted as recommended.**
8. **One tool-to-engine channel:** MOD-75 builds it first (it is not blocked), and MOD-27's consult reuses
   it. **Recommended: accept.**
   **Answered 2026-10-18: accepted as recommended.**
9. **`R-ID-6` and cross-agent text in reads:** another agent's note returned by `item_read` is item work
   (a work product), not a draft; it comes back marked with its author, and unaccepted drafts are never
   returned to another run. **Recommended: accept, and add the sentence of §9.1 to `R-MCP-1`.**
   **Answered 2026-10-18: accepted as recommended.**
10. **Skill resources and `R-ID-5`:** skill text stays inlined; a bound skill's resources (references,
    scripts) may be read on demand through MOD-113's read tool, which needs `R-ID-5` amended (§9.6).
    **Recommended: accept the amendment, scoped to resources of the step's bound skills.**
    **Answered 2026-10-18: accepted as recommended.**
11. **Same-item transcript tail:** add "a scrubbed transcript tail of a named step of the same item" to
    MOD-112 part (1) beside the run listing; other items' transcripts stay out (`R-PRM-1`).
    **Recommended: accept.**
    **Answered 2026-10-18: accepted as recommended.**
12. **MOD-156 (P6) and MOD-157 (P7):** file both as wanted later; **MOD-157** (P7) also waits on MOD-114 (3) and ANA-35.
    **Recommended: accept.**
    **Answered 2026-10-18: accepted as recommended.**
13. **Requirement edits (§9).** **Recommended: accept as written, the wording open to the maintainer's
    edits.**
    **Answered 2026-10-18: accepted as recommended.**

## 9. Requirement text changes proposed

All maintainer-gated; accepted 2026-10-18 (§8 Q13) and applied to `docs/REQUIREMENTS.md` as written,
in a commit of its own. ANA-34 §10.11's coordinator
sentence is folded into 9.2 so `R-MCP-2` is amended once.

**9.1 `R-MCP-1`**, add after "workspace items, documents and requirements": "Read results mark the author
of any model text they return; a run's unaccepted drafts are returned only to that run; an upstream item
outside the active workspace reads as a one-line stub (R-PRM-2)."

**9.2 `R-MCP-2`, proposed text:**

> **R-MCP-2 (must).** Tools: `item_link` (propose an edge, kind), `item_status` (a typed request a person
> accepts), `document_write` (the step's output artifact), `note_add`, `box_profile` read, `command_run`
> and its asynchronous form (start, wait with a deadline, cancel) with reads of the stored command output
> and structured test results, `spawn_subagent` (consult); read tools for items, documents and
> requirements, and semantic search (R-STO-8); `item_create`, which proposes an item as a draft a person
> accepts (R-ID-6); `requirement_cite`, which proposes a citation (R-ENT-15) a person accepts; a question
> to the user that parks the step (R-TUI-11, MOD-75); the planner's tools for declaring parts and next
> steps (R-ORCH-1, R-ORCH-7); and the coordinator's tools (R-ORCH-14): read the runs and waiting rows below
> it, write proposals into the close-out checklist, and request an enqueue or a link through a permission
> card. Any proposal an agent makes that a status move, the scheduler, coverage or a prompt would read is
> held until a person accepts it (R-ID-6). Each session is told which tools it has and when to use them.
> Amended 2026-10-15; extended by ANA-36 and ANA-34.

**9.3 `R-MCP-3`**, replace "and returns output" with "and returns output, or a handle the session waits
on with a deadline; the scrubbed output is stored on the step and readable by the run's sessions. A
project may name command presets with their own timeout."

**9.4 `R-MCP-4`**, replace "Skill text instructs agents" with "The prompt and the MCP server's
instructions tell agents".

**9.5 `R-ENT-9`**, add after "there is no manual link action": "A proposed link of a kind that scheduling,
status or a subgraph reads (`blocked_by`, `supersedes`, `duplicates`, `parent`) is pending until a person
accepts it (R-ID-6)." (Subject to §8 Q2.)

**9.6 `R-ID-5`**, add: "A step's bound skills may carry resources (R-SKL-1) that the agent reads on demand
through `htui`'s MCP read tools; the skill text itself stays inlined." (Subject to §8 Q10.)

**9.7 `R-ENT-15`**, add: "An agent proposes a citation through `requirement_cite` (R-MCP-2), as a draft a
person accepts; amending or withdrawing a requirement is a requirement draft (R-ENT-14)."

## 10. Rejected ideas

**Refuted by the skeptic pass (11):**

- **Close-out proposal (#10):** a composite of `item_status` (resolution), `item_create` (follow-ups),
  `requirement_propose` (wording) and MOD-136 (re-confirmations); MOD-135 already makes the checklist
  deterministic; the evidence is markdown bookkeeping. Folded into **MOD-154** (P4) and MOD-135.
- **`gate_run` (#21):** `R-SEC-2` already resolves the test DSN into the agent's environment
  (`engine.rs:6574-6577`); the classifier denials are Claude Code's, which an htui step does not run
  under; injecting secrets "only into the child" does not keep them from agent-written test code. The
  remainder (presets, directory, timeout) is a `command_run` argument, folded into **MOD-153** (P3).
- **`run_steps_read` (#25):** finished steps reach later steps as input documents (`R-PRM-1`,
  `resolve_inputs`); concurrent siblings have no results to read; planner and integrate views are
  MOD-140's `plan_status`. The two-sessions-in-one-worktree evidence is a TUI concurrency problem.
- **`sibling_runs` (#26):** contention is the queue's per-class limits (`R-MCP-3`); hand-judged ID leases
  vanish with store-minted keys (`R-ENT-7`); coordinator reads are MOD-136's.
- **`step_result` (#27):** `document_write` is already the typed work product, and the engine decides
  completion from deterministic signals; an outcome claim nothing may read adds model text with no
  allowed reader. At most a per-phase schema validated on `document_write`.
- **`notify_user` (#29):** `note_add` already is a non-blocking message to the person; a notify flag
  MOD-114 rings on covers it; the harnesses ship their own.
- **`resource_lease` (#30):** a shared dev database is a command class with limit 1 (MOD-109 settings);
  orphaned children are reaped by the queue already (`command.rs:1-18`).
- **`box_profile` health (#31):** the shell answers `df`, `ps` and `pg_isready`; live probes would break
  the contract that the tool and the prompt's box section say the same thing.
- **`handoff_draft` (#32):** each step is a fresh session fed by documents; continuation is MOD-140's
  `successors` plan or MOD-111's fork from history; the one evidence item is a usage-limit event.
- **`sibling_message` (#33):** not an `R-ID-6` breach (item work), but it breaks `R-PRM-1`'s
  self-contained prompt, needs a live channel htui lacks, and is a cross-agent injection path; no
  transcript demand.
- **`definition_validate` (#36):** no standalone consumer; validation belongs inside the htui agent's and
  the modal's write tools, answering typed refusals as `plan_submit` does.

**Rejected or folded at catalogue time (20), confirmed here:**

- **An htui code search or read tool:** duplicates the harness's own tools and the mandatory code-context
  tools ANA-39 settles (bundles MOD-127/128); the 4,203 hook denials are this dev setup's.
- **An htui todo or progress checklist:** built into every harness; MOD-110 maps those events into rows
  as display; 0 todo calls in the transcripts.
- **`ask_oracle`:** a second, uncapped spawn path; a consult child (MOD-27) is the capped form.
- **`diff_read` / `run_diff`:** the shell has git; htui's extra (the base commit) belongs in the prompt.
- **`store_query` (named or raw SQL):** breaks MOD-11 I-2 and invites the DSN handling the transcripts
  show; the reads are MOD-112's.
- **`workflow_docs_validate` / `docs_check`:** specific to this repo's markdown workflow; htui derives its
  counts deterministically.
- **`project_coordinates`:** project facts for docs, memory (ANA-38) or the box profile, not an htui
  concept.
- **Web fetch and search, file edit, LSP, notebook, cron, sleep, clock, mermaid:** harness built-ins.
- **A planner tool that inserts `run_step` rows:** a second, unfenced writer of `run_step`, breaking
  I-3 (`origin/hr/ANA-33:docs/ANA-33.md:327-334`).
- **Turn-by-turn transfer tools (Swarm, `transfer_to_*`, AutoGen handoffs):** the plan would live in a
  context window with no recovery; `RunKind::Swarm` is dropped (same source).
- **`amend_requirement`:** ANA-11 invariant 2 (`docs/ANA-11.md:267-269`) and `R-ENT-14`.
- **Direct memory writes (Gemini `save_memory`, goose Memory, mem0):** model text would reach future
  prompts unaccepted.
- **A live, tracker-style item create:** `R-ID-6` (`REQUIREMENTS.md:116-118`); `item_create` stays a draft.
- **An `item_status` that moves the status:** MOD-11 I-4 and `R-ENT-8`.
- **A tool that answers another agent's permission or question:** ANA-27 T2.
- **A secret or DSN read tool:** `R-SEC-2`, `CONCEPTS.md:90-91`.
- **Merge, push, PR and forge tools:** ANA-34 §7.1 (the agent never holds a forge credential, never
  merges); MOD-134 keeps those credentials out of agent environments.
- **An agent-run `verify` class:** verify is the orchestrator's gate of record (`command.rs:48-49`);
  agents read its result (**MOD-153** (P3)).
- **A consumer MCP server or an out-of-session `command_run` client:** ANA-29; consumers use MOD-92's
  `--json` CLI.
- **Plan approval that auto-approves on timeout (Jules):** a person must accept.

**Lifecycle tools beside consult spawn** (wait, list, message for subagents) are dropped as separate
tools: the consult answer is a blocking tool result (§7.2, MOD-27).

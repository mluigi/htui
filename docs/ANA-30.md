# ANA-30 - Utility models: auxiliary cheap-model calls (session titles etc.)

> **Scope note:** Survey every place `htui` could hand background or drafting work to a small, cheap
> model (session and item titles, summaries, history digests, commit messages, judge or triage calls,
> and similar), price each use, budget the set against a $100 Claude API credit, and decide whether a
> `utility_model` setting is worth building. Governed by `.claude/rules/workflow-docs.md`,
> `CONCEPTS.md`, `docs/REQUIREMENTS.md`, `docs/ANA-21.md` (model tiers and prices), `docs/ANA-2.md`
> (§4.5 judge, close-out), `docs/ANA-4.md` (billing and caps) and `docs/ANA-5.md` (prompt invariants).
> Origin: HANDOFF item ANA-30 (`HANDOFF.md:79-86`), added 2026-10-13 in `70cb65e4`, the same day as
> `scripts/hr attach --api-key` (`cf4bff43`).
>
> **Requirements addressed:** `R-ID-6`, `R-AGT-7`. Touched at their seams: `R-ID-7` and `R-SEC-2`/`-3`
> (what a utility call may transmit), `R-AGT-5` (model strings are data), `R-AGT-8` (candidate
> order), `R-AGT-9` (htui never holds an agent's credential), `R-ORCH-7` (the judge), `R-ORCH-11` and
> `R-HIS-1` (every model call is recorded), `R-PRM-1` (upstream summaries), `R-ENT-9`/`-10`/`-14`
> (links, item and requirement text), `R-ID-4` (commits), `R-NF-2` (no external daemon).
>
> **Status (2026-10-14): concluded.** The maintainer accepted every §8 recommendation on 2026-10-14.
> No implementation code is written here. The verdict (§7) is **no `utility_model` setting now**:
> every use that survives the requirement check costs single-digit dollars a month on Haiku 5.5 and
> has no surface to appear on yet, and the credit is better spent through a `per_token` registry row
> that the existing mechanisms already support. Spawned: **MOD-97** (an exported `ANTHROPIC_API_KEY`
> silently moves subscription rows onto API billing, §2.3) and **MOD-98** (opt-in `summary` phase,
> U5). Applied in the tree: the `R-ID-6` clarification (call 1, `docs/REQUIREMENTS.md`), the ANA-21
> §5.7 `haiku` amendment (call 8), and `--api-key` in `docs/hr-sandbox.md`. Close-out:
> `docs/decisions/ana/ana-30.md`.

---

## 1. Context and problem statement

The HANDOFF item asks for a survey of "every place htui could use a small, cheap model for
background work: session and item titles, summaries, history digests, commit messages, judge or
triage calls, and similar", with, per use, "the call site, the model tier (`docs/ANA-21.md`), the API
route (direct Claude API vs the `claude` CLI that htui wraps), an estimated token cost, and the
trigger", then a budget against the credit and a decision on "whether a `utility_model` setting is
worth building. A MOD is spawned only if the verdict says yes." (`HANDOFF.md:79-86`).

The motivation is a **$100 Claude API credit** that "is available and needs uses". The item does not
say what kind of credit it is; §3.2 shows that the answer changes both the budget and whether the
one existing consumer competes for it.

The constraint every candidate meets first is `R-ID-6`: "No LLM agent is ever in a sync, cache,
import or bookkeeping path. Those are deterministic code." (`docs/REQUIREMENTS.md:78-79`;
`CONCEPTS.md:36-37` shortens it to "Sync, cache, import and status transitions are deterministic
code"). A use that conflicts is listed and marked, not dropped and not padded in.

The survey ran **five readers in parallel** against HEAD `31b2b317`: the tree's model-using surfaces,
the routes a call can take (driver, CLI, direct HTTP), the requirement and analysis documents that
rule on models in a path, first-hand web pricing, and prior art in other agent tools. A **gap-fill
pass** then reconciled two contradictions between the readers (Haiku 4.5 against Haiku 5.5 as the
price basis, §3.1; a 17k against a 4.2k CLI overhead, §3.3), checked the pinned ACP adapter for a
built-in title, tied the credit's likely identity to this repo's dates, confirmed the environment
hazard on the production spawn path, and produced the aggregate budget the readers lacked.

## 2. Surface as read

Facts from the tree at `31b2b317`.

### 2.1 How htui reaches a model today

- **One entry point: a driver session.** `AgentDriver::start(spec, prompt)` is the only model call
  in the tree. `SessionSpec` requires an `agent_id` and a `step_id` (the `run_step` every recorded
  event belongs to) and carries `env`, `model` (`None` = the agent's default), tools, MCP,
  permission, resume and `budget_micros` (`crates/htui-agent/src/driver.rs:274-307`, `:397-431`). So
  any call through the driver needs a `run` and `run_step` row first, and inherits the recorder's
  scrub, usage and cap.
- **No direct model HTTP client.** `reqwest` serves the installer, the Infisical provider and the
  embedding-model download only (`Cargo.toml:129-135`; `crates/htui-secrets/src/infisical.rs:102`;
  `crates/htui-store/src/model.rs:606-608`). `ANTHROPIC_API_KEY` appears only in tests and in
  `scripts/hr` (`crates/htui-agent/tests/cli_live.rs:927`, `:1021`; `scripts/hr:954-958`).
- **Every seeded row bills a subscription.** "Every row is `billing: subscription`"
  (`crates/htui-core/src/model/agent.rs:115`); the seed test pins it and notes "A box on API-key
  auth flips the row in Settings" (`agent.rs:225-228`); `crates/htui-core/seeds/agent_claude_cli.json:4`.
  The `claude` and `claude-cli` rows seed `models: []` and `default_model: null`
  (`agent_claude_cli.json:5-6`; `agent.rs:122-126`). Model strings are data and no source file reads
  one (`agent.rs:128-132`, `R-AGT-5`).
- **The CLI argv.** `argv` (`crates/htui-agent/src/cli/mod.rs:159-256`) passes `--model` from
  `spec.model` (`:206-208`) and `--max-budget-usd` when a budget is set (`:212-214`); it passes no
  `--effort`. A tool-less session gets `--tools=` and `--strict-mcp-config` (`:238-240`). Every
  isolated session appends `ISOLATION_ARGS` = `--settings {"disableAllHooks":true}
  --disable-slash-commands --setting-sources ""` (`:268-274`), and the row's `extra_args` come last
  (`:254`). An operator's own `--settings` **replaces** htui's rather than merging, so
  `refuse_settings_override` accepts one only if it sets `"disableAllHooks": true`
  (`:284-286`, `:351-388`).
- **Spend is counted only on `run_step.usage`.** The CLI mapper turns `total_cost_usd` into
  per-turn `cost_micros` deltas (`crates/htui-agent/src/cli/claude.rs:412-421`); the recorder
  enforces the per-run cap (`docs/ANA-4.md:1146-1153`). A call outside a `run_step` would have no
  row, no cap and no history (`R-ORCH-11`, `R-HIS-1`, `docs/REQUIREMENTS.md:275-276`, `:284-286`).
- **The judge is already a model call outside the item's own work.** It is a `run_step` at
  `fanout_index = -1`, chosen from `project.settings.judge_agent_id` with **`model: None`**, i.e.
  that agent's default model; `StepGraphPhase` does not carry the `judge_agent_id`/`judge_model`
  columns migration 0003 added (`crates/htui-orch/src/graph.rs:839-849`;
  `crates/htui-store/migrations/0003_orchestration.sql:17-27`; `docs/ANA-21.md:93-95`).
- **Candidate lists are Postgres-only.** Rung 1 of the candidate chain is `phase_agent`; rungs 2
  (`project.settings.default_agent_id`) and 3 (the single enabled agent on the box) yield one
  candidate each (`graph.rs:856-918`). Seeded graphs have no `phase_agent` rows and no UI edits them
  (`docs/ANA-21.md:88-91`; no `phase_agent` reference under `crates/htui/src/ui`), so a second,
  overflow candidate can be listed today only by writing the row in SQL.

### 2.2 The one-shot pattern that already exists (MOD-55 EditHelp)

`StoreRequest::EditHelp` is a user-started, one-turn model call: "The runtime assembles and scrubs
the prompt **before** anything is minted ..., mints a `chat` run whose step is `edit_help`, and opens
a session with no tool lease, every tool kind denied and a deny-all policy. The session ends itself
after its first turn." (`crates/htui/src/store_worker.rs:263-276`). It carries only `agent_id`,
answered "with its default model (PRD D1)" (`:272-273`). The worker mints the run with
`for_edit_help()`, opens no lease and applies `help_policy()` (`crates/htui/src/agent_worker.rs:2419-2456`);
`EDIT_HELP_PHASE = "edit_help"` (`crates/htui-core/src/model/run.rs:291-292`). `HelpTarget` covers
`Template` and `Skill` only (`crates/htui-core/src/prompt/edit_help.rs:9-22`), and
`HelpPrompt::scrubbed` passes name, body and request through `scrub_section` first (`:48-69`).

This is the shape every compliant drafting use below takes: a model **proposes** text, a person
accepts it, the call is a recorded `chat` run, and its input is scrubbed before it is transmitted.
The model it uses is whatever the chosen row defaults to, so a "utility model" is already
expressible as a **row choice**: a registry row whose `default_model` is cheap.

### 2.3 Routes to the API credit, and the key

| Route | What it is | Bills the credit? | Tree support |
|---|---|---|---|
| (a) Direct Messages API | Rust HTTP client, htui-held key | Yes | None (§2.1) |
| (b) `claude -p --model <m>` on the seeded `claude-cli` row | the EditHelp path with a model override | **No**: the row authenticates with the subscription login (plan D92, `agent.rs:225-227`) | Full |
| (c) A dedicated `per_token` row | e.g. a `claude-cli` copy with `--bare`, a pinned model and a key source | Yes | Data only (`R-AGT-5`), key route open (below) |

**The key hazard.** `spawn_supervised` builds the child with `.args(&launch.args).envs(&launch.env)`
and no `env_clear` (`crates/htui-agent/src/launch.rs:1184-1199`), and no crate removes
`ANTHROPIC_API_KEY` (gap-fill search of `env_clear|env_remove` over `crates/`: git isolation, the
editor and tests only). Claude Code's documented precedence ranks `ANTHROPIC_API_KEY` above the
subscription login, and "In non-interactive mode (`-p`), the key is always used when present"
(https://code.claude.com/docs/en/authentication, accessed 2026-10-14). So an `htui` process that
runs with the key in its environment (an `hr attach --shell` sandbox, or an operator who exports it
to try a utility model) moves **every** `claude` and `claude-cli` run onto API billing while their
rows still say `subscription`: the quota latch watches the wrong thing and no per-token cap
applies. A project secret named `ANTHROPIC_API_KEY` would do the same through `SessionSpec.env`
(`driver.rs:283-284`). This is a defect today, independent of ANA-30.

**A key route that fits `R-AGT-9`.** `--bare` reads the key only from `ANTHROPIC_API_KEY` or an
`apiKeyHelper` given in the `--settings` JSON (https://code.claude.com/docs/en/headless, accessed
2026-10-14). A `per_token` row can therefore carry, in `extra_args`, `--bare --settings
{"disableAllHooks":true,"apiKeyHelper":"<command>"}`. That passes `refuse_settings_override`
(`cli/mod.rs:351-388`) because it keeps hooks off, it replaces htui's own `--settings`, which it
restates (`cli/mod.rs:284-285`), and the command reads the key from the box's own keyring at spawn,
so htui never reads, holds or transmits it. **Not verified live:** that the CLI honours
`apiKeyHelper` from flag settings under `--setting-sources ""` and `--bare` together. The tension
with `R-AGT-9`'s "Authentication is a fact about a box, not about a registry row"
(`docs/REQUIREMENTS.md:227-228`) is that the helper *command* lives on the row; the credential it
reads does not (§8 call 4).

**`--bare` may become the `-p` default.** The headless docs say `--bare` "will become the default for
`-p` in a future release" (https://code.claude.com/docs/en/headless, accessed 2026-10-14). The
seeded `claude-cli` row deliberately omits it to use the subscription login (D92;
`crates/htui-agent/tests/cli_live.rs:923-936`), so that release would stop the row authenticating.
Adjacent to ANA-30, recorded here because the route analysis found it.

### 2.4 The requirements every use is checked against

- `R-ID-6` (`docs/REQUIREMENTS.md:78-79`), quoted in §1. ANA-2 invariant 3 enumerates the
  bookkeeping set: "Gate resolution, status moves, overlap admission, isolation setup, commit
  capture, selection bookkeeping and close-out are deterministic code (`R-ID-6`). The judge is the
  single exception `R-ORCH-7` grants, and it decides one thing only: which `fanout_index` wins."
  (`docs/ANA-2.md:114-118`).
- ANA-5 invariant 7: "**No model chooses what enters a prompt.** `R-ID-6`." (`docs/ANA-5.md:149-152`),
  and the ruling that `{{step_summary}}` "is deterministic, not model-written ... re-running a model
  to summarise a step in order to restart that step is both circular and non-reproducible"
  (`docs/ANA-5.md:1301-1304`).
- The one existing permission for model-drafted text: "No model calls in a decision path. No
  `$.model.*` to decide status, links, queue order or a permission answer (`R-ID-6`). A mod may draft
  text a person sends." (`docs/ANA-29.md:490-491`). This document reads `R-ID-6` the same way:
  text a person accepts, and display-only labels that nothing reads back, are outside the rule;
  that reading is §8 call 1.
- `R-ID-7`: "Any transcript or tool output is scrubbed of secrets on the host box before it is
  persisted or transmitted. Scrubbing fails closed." (`docs/REQUIREMENTS.md:80-81`). `R-SEC-2`:
  "`htui`'s own credentials (R-STO-1) are never exposed to a session." (`:314-316`). `R-SEC-3`: the
  scrubber "blocks persistence when an unmasked pattern remains" (`:317-320`).
- `R-AGT-7`: for `per_token` agents "a configurable cap per run and per batch, enforced by
  cancelling the session when exceeded" (`:216-218`). `R-AGT-8`: the first candidate "whose quota
  is not exhausted and whose billing cap is not reached" (`:219-222`). `R-AGT-9`: htui "does not
  read, hold, transmit or store the credential" (`:223-228`).
- `R-ENT-9`: links "are proposed by agents through the `htui` MCP server (R-MCP-2) and by the
  importer (R-LATER-3); there is no manual link action" (`:135-138`). `R-ENT-10`: title and body
  changes "are recorded as revisions with author, box and reason" (`:139-141`). `R-ENT-14`:
  requirement "Text changes only by human action" (`:151`).
- `R-ID-4`: htui "writes no files into managed repositories. The only changes to a working tree are
  made by agents doing the item's work." (`:72-74`).
- `R-NF-2`: "No dependency on any external daemon other than Postgres and the agents." (`:404`).
  ANA-5 rejected Anthropic's `count_tokens` endpoint on it, as "a network call needing an API key on
  the executing box" (`docs/ANA-5.md:803`).

### 2.5 Where text a model could fill sits today

- **Item title and body** are typed in the Backlog item form (`crates/htui/src/ui/tabs/backlog/item_form.rs`)
  and versioned under `R-ENT-10`.
- **Chat titles have no surface.** The Chat tab's title is the constant `"Chat"`
  (`crates/htui/src/ui/tabs/chat/mod.rs:534-536`), `run` has no title column and `item_id` is
  "NULL for free-standing chat" (`crates/htui-store/migrations/0001_init.sql:447-464`; 0003 adds only
  scope and lease columns, `0003_orchestration.sql:40-43`), and the tree reader found no store query
  listing past free-standing chats.
- **ACP sessions may already carry a title.** `session_info_update` lands verbatim in a
  `DriverEvent::Other` row (`crates/htui-agent/src/acp/map.rs:754-771`; `crates/htui-agent/src/event.rs:511`).
  The pinned `claude-agent-acp` v0.75.0 has a `SessionTitles` module that makes one small-model
  `generateSessionTitle` call per session over the last 1,000 characters and emits
  `session_info_update { title }` (https://raw.githubusercontent.com/agentclientprotocol/claude-agent-acp/v0.75.0/src/session-titles.ts,
  accessed 2026-10-14; pin at `crates/htui-core/seeds/agent_claude.json:21-27`). **Not confirmed
  against a live database row.** `claude -p` runs get no generated title
  (https://code.claude.com/docs/en/sessions, accessed 2026-10-14).
- **Upstream summaries are thin.** The prompt reads the latest `document.kind = 'summary'` of each
  upstream item (`crates/htui-store/src/pg/read.rs:542-588`; `crates/htui-core/src/store/traits.rs:146`).
  The only automatic writer is close-out, whose body is a heading, a commit table and one line per
  run (`crates/htui-orch/src/closeout.rs:66-121`). ANA-5 found a missing summary "the common case"
  (`docs/ANA-5.md:87`). ANA-2 already sanctions a model-written summary in one shape: "the last
  step's id when a `close-out` phase produced it" (`docs/ANA-2.md:1486-1491`).
- **The only commit htui writes** is the reconcile merge, subject `htui: reconcile <step>`, which
  plan D146/D147 parse to recognise the step's own merge (`crates/htui-orch/src/isolate/git.rs:1586-1600`).
- **Permission summaries** are the deterministic `"<tool_kind>: <title>"`, scrubbed fail-closed
  (`crates/htui-agent/src/record/relay.rs:840-849`, tree reader).
- **Skill and persona descriptions** are picker one-liners, possibly empty, never rendered into a
  prompt (`crates/htui-core/src/model/skill.rs:68`; `crates/htui-core/src/model/persona.rs:61`, tree
  reader).

### 2.6 The existing consumer: `scripts/hr attach --api-key`

`--api-key` "bills this session to the API: the host's ANTHROPIC_API_KEY is forwarded by name (the
value never lands on a command line)" (`scripts/hr:954-959`). The key reaches the headroom guard
(`:965-968`), the `--shell` login shell (`:970-971`) and, by default, an **interactive**
`claude --dangerously-skip-permissions` (`:973`). Everything started in that container inherits it:
subagents, Claude Code's own background calls, and any `htui` or live test run from the shell (which
then hits the §2.3 hazard). `docs/hr-sandbox.md:151` documents `scripts/hr attach [ITEM] [--shell]`
and does not mention the flag.

## 3. Prices and prior art

### 3.1 Prices (first-hand, 2026-10-14)

Per 1M tokens. Source: https://platform.claude.com/docs/en/about-claude/pricing and
https://platform.claude.com/docs/en/models/haiku-5-5/overview, both accessed 2026-10-14;
Gemini from https://ai.google.dev/gemini-api/docs/pricing, accessed 2026-10-14.

| Model | Input | Output | 5-min cache write | Cache read | Note |
|---|---|---|---|---|---|
| Haiku 5.5 (`claude-haiku-5-5`), prompt ≤ 100K | 0.10 | 0.50 | 0.125 | 0.01 | Released 2026-10-07; prompts above 100K cost 0.50/2.50, priced per request, cache reads counted |
| Haiku 4.5 | 1 | 5 | 1.25 | 0.10 | What ANA-21 priced and tiered |
| Sonnet 5.5 (`claude-sonnet-5-5`) | 2 | 10 | 2.50 | 0.10 | Sonnet 5 also 2/10, now permanent (`docs/ANA-21.md:271-273`) |
| Opus 5.5 (`claude-opus-5-5`) | 4 | 20 | 5 | 0.20 | Thinking cannot be turned off (`docs/ANA-21.md:275-276`) |
| Gemini 3.8 / 3.6 Flash | 0.75 | 3.75 | - | 0.075 | Until 2026-12-31, then 1.50/7.50 |
| Gemini 3.5 Flash-Lite | 0.30 | 2.50 | - | 0.03 | Context-cache storage $1.00 per 1M tokens per hour |

The Batch API halves every Anthropic figure (Haiku 5.5: 0.05/0.25), finishes most batches within an
hour and expires them at 24 hours (https://platform.claude.com/docs/en/build-with-claude/batch-processing,
accessed 2026-10-14); neither `claude -p` nor the ACP adapter can submit one.

Three facts that move the per-use figures:

- **The basis is Haiku 5.5, not Haiku 4.5.** ANA-21's price list (2026-09-28) predates Haiku 5.5
  (`docs/ANA-21.md:265-266`). Claude Code's `haiku` alias now resolves to Haiku 5.5 on the Anthropic
  API from CLI 2.1.293, and to Haiku 4.5 on Bedrock, Vertex and Foundry
  (https://code.claude.com/docs/en/model-config, accessed 2026-10-14). The host CLI is 2.1.296
  (`claude --version`, 2026-10-14). Any utility row must pin `claude-haiku-5-5`, not the alias: the
  alias's per-token price differs tenfold by provider (about 7.7× per call, §5).
- **Thinking is on.** Haiku 5.5 thinks adaptively at default effort `medium`, billed as output, and
  "You can't turn thinking off on Opus 5.5, Sonnet 5.5, Haiku 5.5" inside Claude Code
  (https://code.claude.com/docs/en/model-config, accessed 2026-10-14). htui passes no `--effort`
  (§2.1), so a utility row would carry `--effort low` in `extra_args`. Every estimate below adds an
  **assumed 200 output tokens of thinking at effort low** to short replies (400 to long ones).
- **ANA-21's tier rows are stale for this.** ANA-21 tiers `haiku` as Haiku 4.5 at D = 25 on both
  axes (`docs/ANA-21.md:499`) and says the map "is re-checked when an alias moves" (`:493-494`). It
  has moved. ANA-21's tiers are also quality-only for fan-out slots, with cost excluded by decision
  (`docs/ANA-21.md:351-356`), and have no judge weight (`:347-349`). So the "tier" column in §4
  records the ANA-21 row a use would resolve to and flags it as unscored where it does not apply.

### 3.2 What the $100 most likely is

Since 2026-10-07, Max 5x includes **$100 a month** of API credit (Max 20x $200). It "expires at the
end of each billing cycle" with no rollover, is spent before purchased credit, and at zero "API
requests stop" unless purchased credit or auto-reload exists; nothing is charged to the plan. It
covers the Messages and Batches APIs, the Agent SDK and `claude -p` **run with an API key** from the
linked Console organization; it does **not** cover interactive Claude Code or `-p` signed in with the
plan (https://support.claude.com/en/articles/17154008-monthly-api-credits-for-max-and-team-plans,
accessed 2026-10-14). A purchased credit instead expires a year after purchase and does cover Claude
Code (https://support.claude.com/en/articles/8977456-how-do-i-pay-for-my-claude-api-usage, accessed
2026-10-14).

The amount and the dates fit the monthly credit: launch 2026-10-07, ANA-30 and `--api-key` both
2026-10-13 (`git log`: `70cb65e4`, `cf4bff43`). **This is circumstantial**; the Console's billing
page settles it (§8 call 2). The two readings differ in two ways that matter:

| | Monthly Max credit | Purchased / promotional credit |
|---|---|---|
| Horizon | $100 every month, use it or lose it | $100 once, one year |
| `hr attach --api-key` interactive session | Not covered: does not draw it (whether it is refused or billed to purchased credit is undocumented) | Covered: competes for it |
| `claude -p` children inside that sandbox | Covered: draw it | Covered |
| Overspend | Impossible: hard stop at zero | Possible with auto-reload |

### 3.3 Fixed overhead of a CLI call

The figure measured in this tree is **17,327 total input tokens** for a one-word `claude -p` turn
(input + cache creation + cache read), on CLI 2.1.272, model `claude-opus-5[1m]`, **with tools**
(`crates/htui-agent/tests/estimator_live.rs:32-41`). The docs' "System prompt 4,200 tokens" is an
illustrative simulation of an interactive session (https://code.claude.com/docs/en/context-window,
accessed 2026-10-14), not a measurement. A `--bare --tools=` call is smaller by an unmeasured amount.
This document uses 17,327 as an **upper bound**. On Haiku 5.5 it costs **$0.0022** as a cold
5-minute cache write (17,327 × $0.125/M) and $0.00017 as a warm read; API-key `-p` gets a 5-minute
TTL (https://code.claude.com/docs/en/prompt-caching, accessed 2026-10-14), so utility calls more than
five minutes apart pay the cold figure. For a title-sized call the overhead is about 85% of the
cost.

### 3.4 Prior art

All sources accessed 2026-10-14; URLs in §9.

| Tool | Cheap-model uses | Setting shape | Default when unset | Opt-out |
|---|---|---|---|---|
| Claude Code | Session titles, `--resume` summaries, `/usage` status ("typically under $0.04 per session") | One env var, `ANTHROPIC_DEFAULT_HAIKU_MODEL` (`ANTHROPIC_SMALL_FAST_MODEL` deprecated) | The `haiku` alias; on third-party providers the main model | `CLAUDE_CODE_DISABLE_TERMINAL_TITLE=1` for titles; `-p` runs get none |
| aider | Commit messages, chat-history summarisation | One key, `--weak-model` / `weak_model_name` per main model | Per-model pairing (Sonnet/Opus → Haiku), else the main model; `commit_message_models` = `[weak, main]` | None: the fallback is the main model |
| opencode | Session titles; hidden compaction and summary agents | One key, `small_model`, plus `agent.title.model` | `Model.small(primary provider)` (first Haiku/Flash family), then the primary | Disable the title agent (reported ignored, #54917) |
| Zed | Commit messages and thread summaries; inline assist and compaction are separately configurable | One `{provider, model}` key per feature | Commit messages and thread summaries: provider's `default_fast_model` (Anthropic: the lexicographically greatest `claude-haiku-*` id, roughly the newest, else any `claude-*`), then `default_model`; inline assist falls straight back to `default_model`; compaction has no fallback at all | Per feature, by setting the main model |
| OpenAI Codex CLI | Memory extraction and consolidation, `/review`, approval review | One key per feature | Provider-preferred model (`gpt-5.6-luna` at low effort for extraction) | Per feature |
| GitHub Copilot (VS Code) | Chat titles, commit messages, PR title and description | None: server-chosen `copilot-fast` family | Server's choice | None |
| Cursor | Auto-review classifier only (Gemini 3.5 Flash Lite) | None | Fixed | None documented |
| Cline, Roo Code | None: commit messages on the main model with reasoning off (Cline); condensing deliberately on the active model (Roo) | - | - | - |
| gptme, Goose | Conversation names | gptme: provider summary model; Goose: the session's own model | gptme: none on failure; Goose `cursor_agent`: first four words, no model | - |
| Vibe Kanban, Conductor, Claude Squad | PR title from the task name; AI PR description opt-in per PR; Conductor asks the main agent to rename the branch; Squad none | None | Deterministic | Per PR |

What the table says for htui:

- **A setting is one global key or one key per feature, and the fallback is always the main
  model.** Zed and opencode default to the cheap model of the *primary's own provider*, so content
  does not reach a second vendor unless the user configures it.
- **Orchestrators comparable to htui keep labels deterministic** (task name → PR title, diff
  statistics → workspace summary) and make model drafting opt-in per action.
- **The reported failures are htui's own requirements in other tools' bug trackers.** opencode's
  title request skipped the redaction plugin and sent the raw first message (#46115), built titles
  from injected memory context (#23114), sent attachments (#50654), saved a raw `<think>` tag as the
  title (#53493), ignored its disable flag (#54917), and logged phantom usage lines (#52367); aider
  ignored its weak-model override for commits (#5213). Each maps to `R-ID-7`/`R-SEC-3` (scrub
  before transmit), output validation, and `R-AGT-7` (every call counted).
- **Anthropic itself does not use Haiku for everything.** The auto-mode permission classifier runs
  on Sonnet 5, and compaction uses the session model, with a request for a compaction-model setting
  still open (anthropics/claude-code #96316).

## 4. Candidate uses

Assumptions for every row: Haiku 5.5 prices (§3.1); route (b)/(c) CLI with the 17,327-token cold
overhead (§3.3) unless stated; thinking allowance per §3.1; volumes are a single developer's upper
estimate from the tree reader's figures. Route letters are §2.3's. "Tier" is the ANA-21 row the
model would resolve to; Haiku 5.5 has no ANA-21 row.

| # | Use | Call site | What exists today | Tier (ANA-21) | Route | Est. in / out tokens | Est. $ per call | Trigger | Est. frequency | `R-ID-6` / requirement verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| U1 | Item title suggestion | New `HelpTarget::Item` (`edit_help.rs:11`) in the item form (`item_form.rs`), via `StoreRequest::EditHelp` (`store_worker.rs:269`) | User-typed; EditHelp covers templates and skills only | `haiku` D/25 (Haiku 4.5; stale) | (c) | 2k / 40 + 200 | 0.0025 | User keystroke in the item form | 5-20/day | **Allowed** as a suggestion the user saves (revision author stays the human, `R-ENT-10`); an automatic write is not |
| U2 | Item body draft | Same as U1 | User-typed | as U1 | (c) | 3k / 1,000 + 200 | 0.0031 | User keystroke | 0-5/day | **Allowed** as U1 |
| U3 | Chat title, ACP rows | Reader over `other` rows with `update = 'session_info_update'` (`acp/map.rs:754-771`); shown in a chat list not yet built | Generated by the adapter, stored, never read; tab says `"Chat"` (`chat/mod.rs:534-536`) | n/a | none: the adapter's own call, on the subscription | 0 / 0 for htui | **0** | ACP turn end, once per session | 5-15/day | **Allowed**, display-only; not an htui model call; stored rows are already scrubbed |
| U4 | Chat title, `claude-cli` row | One-shot after the first `Done` of a `kind = 'chat'` run; no title column (`0001_init.sql:447-464`) | Absent | as U1 | (c) | 2k / 20 + 200 | 0.0025 | End of first turn | 5-15/day | **Allowed**, display-only, input from scrubbed `session_event` rows; a first-words label is the free alternative |
| U5 | Prose summary for upstream readers (`R-PRM-1`) | A `summary`/`close-out` graph phase whose candidate is a cheap row (`docs/ANA-2.md:1490-1491`); read by `pg/read.rs:588` | Close-out's commit table, no prose (`closeout.rs:66-121`) | `haiku` 25 or `*` 65 (Sonnet) | ordinary phase on (c) | 30k / 800 + 400 | 0.0058 (Haiku 5.5); 0.115 (Sonnet 5.5) | Graph reaches the phase | 2-10/day | **Allowed as an agent phase. Conflicts** if put inside close-out (ANA-2 inv. 3) or generated at prompt assembly (ANA-5 inv. 7) |
| U6 | Cheap fan-out judge | `project.settings.judge_agent_id` naming a cheap row; `graph.rs:841-848` uses its default model | Judge on the named agent's default model | no judge weight (`docs/ANA-21.md:347-349`) | ordinary judge step on (c) | 2 × 60k / 600 + 300 | 0.017 per judged point (two calls, `docs/ANA-18.md:119`, each paying its own cold overhead: 2 × ($0.0022 + 60k × $0.10/M) + 900 × $0.50/M) | Ungated fan-out with ≥ 2 survivors | 0-5 points/day | **Allowed** (`R-ORCH-7`); quality and same-family bias risk (`docs/ANA-21.md:161`); keep each prompt < 100K: one 120k prompt plus overhead would be billed whole at the > 100K rate (§3.1), about $0.073 a point |
| U7 | EditHelp (templates, skills) on a cheap row | `agent_help.rs`, `StoreRequest::EditHelp` | Exists, on the chosen row's default model (subscription) | as U1 | (c) instead of (b) | 6k / 2,000 + 200 | 0.0039 | Ctrl+G in a Skills editor | 0-5/day | **Allowed** (exists; user-reviewed diff) |
| U8 | Skill and persona description one-liners | Skills Library editor, Settings › Personas (`skill.rs:68`, `persona.rs:61`) | User-typed or imported, may be empty | as U1 | (c) | 3k / 30 + 200 | 0.0026 | User action | rare | **Allowed** on user action; **conflicts** if run inside `SKILL.md` or persona import (`R-ID-6` "import") |
| U9 | Triage suggestion (kind, priority, required tags, touched paths) | Item form, validated by `item_spec.rs` | User picks every field | as U1 | (c) | 4k / 100 + 200 | 0.0027 | User asks while minting | 5-20/day | **Needs a carve-out reading**: these fields drive admission, capability refusal and overlap (`R-ORCH-6`/`-9`/`-10`); acceptable only pre-filled into a form the user saves |
| U10 | One-line digest per run step (display) | Runs tab (`ui/tabs/backlog/detail/runs.rs`); a separate asynchronous writer | Absent; full replay (`R-HIS-2`) | as U1 | (c) | 20k / 60 + 200 | 0.0043 | Step terminal, or on open | 10-50/day automatic | **Allowed while display-only**; **conflicts** if it ever feeds a prompt (`R-PRM-1`, ANA-5 inv. 7) or runs inside the recorder or cache refresh (`R-ID-6`). A deterministic digest (ANA-5 §4.6 shape) comes first |
| U11 | Plain-words explanation of a permission request | `record/relay.rs:840-849`, waiting-on-you overlay | Deterministic `"<tool_kind>: <title>"` | as U1 | (c) | 2k / 60 + 200 | 0.0025 | Request parked | 10-100/day | **Not recommended**: adds a model and latency to the security path and sends tool arguments to a second model; not a decision, so not an `R-ID-6` conflict |
| U12 | Draft of requirement text, rationale or a gate reject note | Requirements tab forms; `run_step.gate_note` | User-typed | as U1 | (c) | 3k / 200 + 200 | 0.0027 | User action | rare | **Allowed** as a draft (`R-ENT-14`: text changes only by human action, which accepting is) |
| X1 | Model-written handoff `{{step_summary}}` | `crates/htui-core/src/prompt/render.rs` step_summary | Deterministic from `session_event` | - | - | - | - | Promote without resume | rare | **Conflicts**: rejected by name (`docs/ANA-5.md:1301-1304`) |
| X2 | Model ranking of file excerpts | `ExcerptProvider` | Fixed-weight ranker | - | - | - | - | Every prompt | per step | **Conflicts**: ANA-5 inv. 7 (`docs/ANA-5.md:149-152`) |
| X3 | Commit messages | `isolate/git.rs:1587-1589` | Fixed `htui: reconcile <step>`; agents write their own commits | - | - | - | - | Reconcile merge | - | **Conflicts**: the subject is a parsed marker (D146/D147) in commit capture (ANA-2 inv. 3); htui committing agent work breaks `R-ID-4` |
| X4 | Duplicate detection / link proposals | `item_link`; beside the `R-STO-8` search | Agents via MCP in a run step, or the importer | - | - | - | - | Item mint | - | **Conflicts**: `R-ENT-9` allows no other writer; display-only "possible duplicates" from Qdrant needs no model |
| X5 | Automatic triage, queue order, permission answers | Queue runner, `select`, relay | Code or a person | - | - | - | - | - | - | **Conflicts**: `R-ID-6`; `docs/ANA-2.md:1550-1554`; `docs/ANA-29.md:490-491` |
| X6 | Model-assisted legacy import (`R-LATER-3`) | Importer | Deterministic parse | - | - | - | - | - | - | **Conflicts**: "import" is named in `R-ID-6` |

Not a utility use, but the only consumer that can spend the credit, and so the comparison §5 needs:

| # | Use | Call site | Route | Est. per call | Trigger | Verdict |
|---|---|---|---|---|---|---|
| O1 | `per_token` overflow row for ordinary phase work | A `claude-cli` copy (`--bare`, pinned `claude-sonnet-5-5` or `claude-opus-5-5`, key via `apiKeyHelper`), listed after the subscription row in `phase_agent` | (c) | ~$1.25 per Sonnet 5.5 phase run, ~$2.50 per Opus 5.5 run (estimate: 1.5M cache reads + 0.2M cache writes + 60k output; not measured) | Subscription quota latched (`R-AGT-8`) | **Allowed**: ordinary agent work under `R-AGT-7` caps; needs `phase_agent` rows written in SQL (§2.1) |

### 4.1 Reasoning per use

**U1, U2 (item title and body).** The cleanest fit: EditHelp already does scrub-before-mint, a
deny-all one-turn session and a reviewed proposal. The missing piece is a `HelpTarget::Item` and the
form wiring, not a model setting. Value is modest: a person writing the body usually knows the
title.

**U3 (ACP chat titles).** Strictly better than any utility call for the `claude` row: the title is
already generated by the adapter on the subscription and stored. What is missing is a place to show
it, because htui has no list of past free-standing chats (§2.5). Live confirmation that the rows
exist is §8 call 6.

**U4 (`claude-cli` chat titles).** Real cost only on the degraded CLI row. Prior art (Goose's local
naming, Cline's deterministic fork titles) shows a first-words label is an acceptable free default;
a model title is an upgrade, not a need.

**U5 (summaries).** The highest-value use: upstream prompts today receive a commit table or "no
summary yet". It is also the one use that needs nothing new: a phase in the graph is ordinary item
work on a registry row, recorded as a `run_step`, which ANA-2 already sanctions. Haiku 5.5 makes it
almost free; Sonnet 5.5 is the safer quality choice at about 20× the price. It lengthens every graph
by one step, which is the maintainer's call (§8 call 5).
*Amended by ANA-33, 2026-10-18 (`docs/ANA-33.md` §7.12, `docs/decisions/ana/ana-33.md`):* "lengthens every
graph by one step" becomes "adds one node to the graphs that opt in; other graphs are unchanged". Opt-in is
a seeded alternative graph per project and a library workflow, with no disabled node (MOD-98).

**U6 (cheap judge).** Possible today with no code: name a cheap `per_token` row in
`project.settings.judge_agent_id`, and the judge runs on its default model (`graph.rs:841-848`). A
per-phase judge model needs the H-14 plumbing. ANA-18 already found judge cost negligible
(`docs/ANA-18.md:108-127`) and a weak judge picks worse implementations, so the saving is cents and
the risk is quality.

**U7 (EditHelp on a cheap row).** Today it costs nothing extra on the subscription; moving it buys
quota headroom, not dollars (§5).

**U8, U12 (descriptions, drafts).** Negligible cost and negligible value. Listed so the survey is
complete.

**U9 (triage suggestion).** The fields it would fill are the scheduling inputs `R-ID-6` exists to
protect. A suggestion the user saves keeps the decision human, but a wrong `touched_paths` silently
serialises or parallelises runs, so a person must read it, not just accept it. Not worth building
before a user asks for it.

**U10 (step digests).** The largest utility line in the budget (§5) and the one closest to a
bookkeeping path: it must be a separate asynchronous writer, display-only, and never read by a prompt.
ANA-5 §4.6 shows a deterministic digest from `session_event` counts and the last assistant tail is
feasible; that should be tried first.

**U11 (permission explanations).** Rejected on merit: it puts tool arguments in front of a second
model on the security path for a label the user can already read.

**X1-X6.** Each is ruled out by an existing requirement or analysis, cited in the table. None is
proposed.

**What duplicates an agent-written document.** U5 as a background call would duplicate what a
`summary` phase writes; U10 would duplicate the replay and, if fed forward, the step's own output
document. Only the phase shape of U5 is kept.

## 5. Budget against the $100 credit

**Per call** (from §4's table, Haiku 5.5, CLI cold overhead $0.0022 included):

| Use | $ per call | Upper volume per day | $ per day |
|---|---|---|---|
| U1 item title | 0.0025 | 20 | 0.050 |
| U2 item body | 0.0031 | 5 | 0.016 |
| U3 ACP chat title | 0 | 15 | 0 |
| U4 CLI chat title | 0.0025 | 15 | 0.038 |
| U5 summary phase (Haiku 5.5) | 0.0058 | 10 | 0.058 |
| U6 judge (per point, two calls) | 0.017 | 5 | 0.085 |
| U7 EditHelp | 0.0039 | 5 | 0.020 |
| U8 descriptions | 0.0026 | 2 | 0.005 |
| U9 triage suggestion | 0.0027 | 20 | 0.054 |
| U10 step digests | 0.0043 | 50 | 0.215 |
| U12 drafts | 0.0027 | 2 | 0.005 |
| **Total** | | | **0.55** |

**Per month** (30 days): **$16** at the upper volume with every allowed use on; **$9.9** without U10;
about **$4** at a quarter of the upper volume (assumption: a typical day). Variations:

- Pinning **Haiku 4.5** (or using the `haiku` alias on Bedrock or Vertex) costs ten times as much
  per token, but Haiku 5.5's newer tokenizer counts the same text as about 30% more tokens than
  Haiku 4.5 (https://platform.claude.com/docs/en/models/haiku-5-5/overview, accessed 2026-10-14),
  and the token estimates here, the 17,327 overhead included (measured on Opus 5, same tokenizer),
  are in that newer count. Per call the multiplier is about 10 / 1.3 ≈ 7.7×: about **$126/month**
  upper, $76 without U10. The 200-token thinking allowance may not apply either, since an omitted
  `thinking` field meant no thinking on Haiku 4.5 (anthropics/claude-code#100775), which would
  lower it slightly further.
- Running U5 on **Sonnet 5.5** instead adds 10 × ($0.115 − $0.0058) = $1.09/day: **$49/month**.
- A **direct API** route (option C) removes the cold overhead from 139 calls a day (the U1-U12 call
  counts above, two judge calls per point): 139 × $0.0022 = **$0.31/day, about $9.20/month saved** at the
  upper volume. A title becomes about $0.0003 (2k × $0.10/M + 240 × $0.50/M).

**Months of credit.**

- If the $100 is the **monthly Max credit** (§3.2): the utility uses consume **4-16%** of each
  month's pot at Haiku 5.5, and the remaining $84-96 lapses unspent every month.
- If it is a **one-off** credit: $100 / $16 ≈ **6 months** at the upper volume, ~24 months at the
  typical one, so most of it expires at the one-year mark.

**Compared with the sandbox consumer.** Anthropic reports an average of about **$13 per developer
per active day** for Claude Code, under $30 for 90% of users (https://code.claude.com/docs/en/costs,
accessed 2026-10-14). If `hr attach --api-key` draws on the credit (purchased reading), one active
sandbox day spends as much as **24 days** of every utility use at the upper volume ($13 / $0.55),
and $100 lasts roughly 3-8 sandbox days (the lower bound assumes Opus 5.5 with workflow fan-out at
$30/day). Under the monthly reading the interactive session does not draw on it at all, but any
`claude -p` child inside the sandbox does.

**Compared with the overflow row (O1).** $100 buys about **80 Sonnet 5.5 or 40 Opus 5.5 phase runs**
(estimate, §4). That is work htui already does, moved off the subscription quota when it latches.

**Conclusion.** The utility uses would consume only about $4-16 a month on Haiku 5.5, so they cannot
be what the credit is for. The credit is better spent as `per_token` headroom for real phase work
(O1) and, if it is purchased rather than monthly, on `hr --api-key` sandbox sessions, which spend it
two orders of magnitude faster than any utility use. Where utility calls do matter is **quota, not
dollars**: today EditHelp and the judge draw from the same opaque 5-hour and weekly subscription
window as coding steps (`docs/ANA-21.md:276-278`), and a `per_token` Haiku row would move them off it.

## 6. Options

| | (A) No utility model | (B) `utility_model` setting reusing the CLI driver | (C) Direct Messages API client | (D) Registry row with a `utility` role |
|---|---|---|---|---|
| **Shape** | Nothing new. Cheap work is a registry row the user picks (EditHelp's agent picker, `judge_agent_id`, `phase_agent`) | An `app_setting` key `utility` = `{agent_id, model}` (`0001_init.sql:557-561`, no migration); consumers default to it; calls stay EditHelp-shaped `chat` runs, `claude -p --model <pinned id> --effort low` | A Rust client over `reqwest` + `json`, a keyring slot under service `htui`, a model id and price table as data | `agent.settings.roles: ["utility"]` (JSONB, no DDL) or a `role` column (pg + cache migration); the first enabled row on the box with the role serves |
| **Cost per title** | n/a | ~$0.0025 cold, ~$0.0005 warm | ~$0.0003; Batch halves it | As (B) on a Claude row; Gemini Flash rows cost more per token (§3.1) and `agy` is a subscription |
| **Wiring** | None. Optionally seed a disabled `claude-api` `per_token` row | Small to medium: `htui-core` (setting key, `HelpTarget::Item`), `htui` (`store_worker.rs`/`agent_worker.rs` default agent, item form, Settings field). The Settings field is not free: the editable `app_setting` keys come from a closed, typed registry (`SettingKey`, twelve keys, `crates/htui-core/src/prompt/settings.rs:155-189`) whose `SettingKind` is only `Integer`, `Fraction` or `Boolean` (`:302-314`), so an object `{agent_id, model}` has no kind, validator or editor there. The object-valued keys live outside it: `command_limits` is seeded in SQL (`0003_orchestration.sql:129`) with no editor, and `box_probe_spec` got its own JSON editor in `Settings > Boxes` (`crates/htui-agent/src/box_probe/spec.rs:13-28`). `utility` needs a new kind or a dedicated editor of that sort. Two to three crates, a handful of files; still no Postgres migration | Large: new client module, keyring slot and Settings field (`htui-store`/`htui`), scrub before send and before persist, a usage home (a `run_step` row anyway, or a new table, since spend is summed only from `run_step.usage`), its own cap. ANA-18 calls this "a third transport, a driver, and an enum migration" (`docs/ANA-18.md:95-101`) | Medium: role parse in `htui-core`, a resolver in `htui`, the Settings editor; a column adds two migrations |
| **`R-ID-6`** | Fits | Fits if consumers are suggestions and display labels only | Fits for the same consumers | Fits as (B) |
| **`R-AGT-5`** | Fits | Fits: `{agent_id, model}` is data | Outside the registry: a model call no row describes | Fits best: a row plus data |
| **`R-AGT-9`, `R-SEC-2`, `R-STO-1`** | Fits | The key must reach the child: `apiKeyHelper` (§2.3), never htui's env | htui holds a new credential (keyring per `R-STO-1`; never given to a session per `R-SEC-2`); an agent credential it is not | As (B) |
| **`R-NF-2`, `R-HIS-1`, `R-AGT-7`** | Fits | Fits: a recorded `chat` run under the recorder's cap | **Strains `R-NF-2`** (ANA-5:803 precedent); recording and caps must be rebuilt | Fits |
| **Failure: key absent or call fails** | n/a | Setting unset or row not enabled on this box → today's behaviour (the user picks the row); call fails → the form keeps the user's text, the label falls back to a deterministic one (first words, item key), shown as a chat `Failed` frame | Key absent → feature hidden; HTTP error → deterministic text; scrub refusal → no call | No row with the role on this box → as (B) unset |
| **Credit spend** | Via O1 only | Via a `per_token` row | Directly | Via a `per_token` row |

Every option must degrade to deterministic text: no use in §4 gates anything, so a missing or failed
call leaves the user's own text, the item key, or a first-words label, and nothing waits on it.

## 7. Verdict and phasing

**A `utility_model` setting is not worth building now. Spawn no MOD for it.** Three reasons:

1. **Nothing would read it.** The two uses worth having, item-title drafting (U1) and chat titles
   (U3/U4), are blocked on surfaces that do not exist: `HelpTarget` has no item variant and htui has
   no list of past chats. EditHelp already carries an `agent_id` per request, so the only existing
   consumer can use a cheap row by being pointed at it.
2. **It is not the way to spend the credit.** At Haiku 5.5 every allowed use together is about
   $4-16 a month (§5). The credit is spent by a `per_token` row doing real work (O1), which needs no
   code: `R-AGT-7`/`-8` already cap and order it, and ANA-4 already says "a box using API-key auth
   flips the row in Settings" (`docs/ANA-4.md:997`).
3. **The direct-API route (C) runs against two settled precedents** (`R-NF-2` via
   `docs/ANA-5.md:803`; ANA-18's third-transport cost) for a saving of about $9.20 a month at the
   upper volume.

**What to do with the credit instead**, in order:

1. Settle which credit it is (§8 call 2).
2. Add a disabled-by-default `per_token` row, e.g. `claude-api`: a `claude-cli` copy with
   `default_model` pinned to a full id, `extra_args` `--bare`, `--effort low` for a Haiku row, and
   `--settings {"disableAllHooks":true,"apiKeyHelper":"<box-local command>"}`; verify the
   `apiKeyHelper` route once live (§8 call 4). This is a registry row (`R-AGT-5`), entered in Settings
   or SQL, not a MOD.
3. Point it where it pays: as the second `phase_agent` candidate on `implement`/`fix` (overflow when
   the subscription latches, O1), as `judge_agent_id` if a cheap judge is wanted (U6), and as the
   agent picked in EditHelp (U7). Set `per_token_cap_run` so one run cannot take the month.
4. If the credit is purchased rather than monthly, it is also fair game for `hr attach --api-key`
   sandbox days, which spend it fastest.

**Recommended regardless of the verdict** (not utility-model MODs):

- **MOD-97: stop an inherited `ANTHROPIC_API_KEY` rerouting subscription rows.**
  Strip vendor auth variables (`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`) from the spawn
  environment of rows with `billing = subscription`, or refuse the spawn naming the variable
  (`launch.rs:1184-1199`, §2.3). Filed as MOD rather than CLEAN because it changes behaviour.
- **ANA-21 amendment: the `haiku` row has moved** to Haiku 5.5 (§3.1); re-tier or re-key it on the
  full model id, per ANA-21's own re-check rule (`docs/ANA-21.md:493-494`). Applied as a dated
  amendment under ANA-21 §5.7.
- **Track the `--bare`-by-default change** against the seeded `claude-cli` row (D92, §2.3). Folded
  into MOD-97.
- **Document `--api-key`** in `docs/hr-sandbox.md:151`, with which credit it draws on. Done.

**What reopens the verdict.** The first MOD that adds a **second model-drafting surface** (a
`HelpTarget::Item`, or a chat-history list that needs titles on `claude-cli` rows). At that point the
setting is option (B), built inside that MOD rather than as its own: one `app_setting` key
`utility = {agent_id, model}`, read as the default agent for every drafting surface, falling back to
the user's pick when unset or not enabled on the box, and to deterministic text when the call fails.
Two further triggers: the subscription quota latching often enough that EditHelp or the judge are
refused (the quota argument of §5), or a use appearing whose volume makes the CLI overhead matter
(more than about 1,000 calls a day, where (C)'s saving passes $2/day).

**Phasing, if the maintainer overrides to "yes now".** One MOD-<new>: "utility agent setting:
`app_setting` `utility` = `{agent_id, model}`, `HelpTarget::Item` for title and body drafts in the
item form, deterministic fallback, scrub-before-mint as EditHelp". Chat titles follow in the MOD that
builds the chat history list, reading U3's stored `session_info_update` first and calling the
utility row only for `claude-cli` sessions.

## 8. Open calls for the maintainer

**Settled 2026-10-14: the maintainer accepted every recommendation below.** Calls 1 and 8 are applied
in the tree, 5 and 7 are MOD-98 and MOD-97, and 2, 4 and 6 are checks owed on the host or in the first
MOD that wires a `per_token` row; 3 and 9 need nothing.

1. **Does `R-ID-6` cover display-only derived labels and drafts a person accepts?** No requirement
   rules on it; ANA-29's "A mod may draft text a person sends" (`docs/ANA-29.md:491`) is the nearest
   precedent. **Recommended: no, they are outside it**, on the condition that no such text is ever
   read by a status transition, a link, an import, the cache refresh or a prompt. Record the reading
   as a one-line clarification under `R-ID-6`.
2. **Which credit is the $100: monthly Max credit or purchased?** **Recommended: check Console ›
   Settings › Billing before spending it.** If monthly, treat it as use-it-or-lose-it headroom for
   O1 and do not route sandbox sessions to it; if purchased, decide how much to reserve for
   `hr --api-key` sandbox days first.
3. **Build a `utility_model` setting now?** **Recommended: no** (§7); build option (B) inside the
   first MOD that adds a second drafting surface.
4. **Is an `apiKeyHelper` command on a registry row compatible with `R-AGT-9`** ("Authentication is
   a fact about a box, not about a registry row")? **Recommended: yes**, because the row holds a
   command name and each box's own keyring holds the credential; htui never reads it. Verify once
   live that `--bare` honours `apiKeyHelper` from flag settings under `--setting-sources ""` before
   relying on it.
5. **Fill upstream summaries with a `summary` phase on a cheap row (U5)?** **Recommended: yes, as an
   opt-in phase in the `feature` and `bug` graphs**, Sonnet 5.5 by default for quality (~$0.12 per
   item), Haiku 5.5 where cost matters. It is ordinary item work and needs no utility setting.
6. **Confirm ACP chat titles (U3) live.** **Recommended: one query on the dev database** for `other`
   rows with `update = 'session_info_update'`, before any chat-title design calls a model.
7. **Spawn the `ANTHROPIC_API_KEY` env-strip item now?** **Recommended: yes**, as CLEAN-<new>, since
   the hazard exists today in every `hr attach --shell` session that runs `htui`.
8. **Re-tier ANA-21's `haiku` row?** **Recommended: yes**, re-key it on `claude-haiku-5-5` and leave
   it unscored (resolving through `*`) until a source scores Haiku 5.5, as ANA-21 did for `sonnet`
   (`docs/ANA-21.md:569-571`).
9. **Should a cheap judge (U6) be used at all?** **Recommended: not yet.** Judge cost is already
   negligible (`docs/ANA-18.md:108-127`) and same-family bias is documented (`docs/ANA-21.md:161`);
   revisit with MOD-36's mixed-model data.

## 9. Sources

Tree: `path:line` references inline in §2 and §4, at HEAD `31b2b317`.

All web sources accessed **2026-10-14**. Figures re-derived from them are marked as estimates in the
text.

**Prices and credit.**

- Anthropic pricing: https://platform.claude.com/docs/en/about-claude/pricing
- Haiku 5.5 overview and migration guide: https://platform.claude.com/docs/en/models/haiku-5-5/overview,
  https://platform.claude.com/docs/en/models/haiku-5-5/migration-guide
- Batch processing: https://platform.claude.com/docs/en/build-with-claude/batch-processing
- Monthly API credits for Max and Team: https://support.claude.com/en/articles/17154008-monthly-api-credits-for-max-and-team-plans
- Paying for API usage: https://support.claude.com/en/articles/8977456-how-do-i-pay-for-my-claude-api-usage
- Credit terms: https://www.anthropic.com/legal/credit-terms
- Agent SDK with a Claude plan (2026-10-07 entry): https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan
- Gemini API pricing: https://ai.google.dev/gemini-api/docs/pricing. `gemini-3.7-flash`, `agy`'s
  seeded default, has no section on that page today; whether it was withdrawn is **unverified**.

**Claude Code behaviour.**

- Authentication precedence: https://code.claude.com/docs/en/authentication
- Headless and `--bare`: https://code.claude.com/docs/en/headless
- Model configuration, aliases, thinking: https://code.claude.com/docs/en/model-config
- Costs and background usage: https://code.claude.com/docs/en/costs
- Session titles: https://code.claude.com/docs/en/sessions
- Prompt-caching TTLs: https://code.claude.com/docs/en/prompt-caching
- Context-window simulation (the 4,200 figure): https://code.claude.com/docs/en/context-window
- Environment variables: https://code.claude.com/docs/en/env-vars
- `claude-agent-acp` session titles at the pinned tag:
  https://raw.githubusercontent.com/agentclientprotocol/claude-agent-acp/v0.75.0/src/session-titles.ts
- Issues: https://github.com/anthropics/claude-code/issues/96316 (compaction model),
  https://github.com/anthropics/claude-code/issues/100775 (Haiku 5.5 thinking through the CLI).

**Prior art.**

- aider: https://aider.chat/docs/config/options.html,
  https://raw.githubusercontent.com/Aider-AI/aider/main/aider/models.py, issue
  https://github.com/Aider-AI/aider/issues/5213
- opencode: https://opencode.ai/docs/config/, https://opencode.ai/docs/agents/,
  https://raw.githubusercontent.com/anomalyco/opencode/dev/packages/core/src/session/title.ts; issues
  #46115, #23114, #50654, #53493, #54917, #54692, #52004, #52367 under
  https://github.com/anomalyco/opencode/issues
- Zed: https://zed.dev/docs/ai/agent-settings,
  https://raw.githubusercontent.com/zed-industries/zed/main/crates/language_model/src/registry.rs
- OpenAI Codex CLI: https://learn.chatgpt.com/docs/config-file/config-reference,
  https://raw.githubusercontent.com/openai/codex/main/codex-rs/model-provider/src/provider.rs
- GitHub Copilot Chat: https://github.com/microsoft/vscode-copilot-chat/blob/main/src/platform/endpoint/common/endpointProvider.ts
- Cursor: https://cursor.com/docs/models
- Cline: https://raw.githubusercontent.com/cline/cline/main/apps/vscode/src/hosts/vscode/commit-message-generator.ts;
  Roo Code: https://roocodeinc.github.io/Roo-Code/features/intelligent-context-condensing
- gptme: https://raw.githubusercontent.com/gptme/gptme/master/gptme/util/auto_naming.py; Goose:
  https://raw.githubusercontent.com/aaif-goose/goose/main/crates/goose/src/session/session_naming.rs
- Vibe Kanban: https://vibekanban.com/docs/workspaces/git-operations.md; Conductor:
  https://www.conductor.build/docs/concepts/workspaces-and-branches; Claude Squad tree:
  https://api.github.com/repos/smtg-ai/claude-squad/git/trees/main?recursive=1

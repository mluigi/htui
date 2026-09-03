# HANDOFF - Outstanding Work (htui)

> **Purpose:** Carry outstanding work between sessions so work resumes.
> `htui` is a Rust TUI that wraps coding agents (claude, agy) to manage the handoff workflow
> (`handoff-run` / `handoff-add` equivalents) across repos and boxes. Items live as a thin
> `items.json` (uuid + key + status) in each managed repo, with the shared source of truth in
> Postgres and the story mirrored to OneDev issues.

## How to use this file

- **Session start:** read this file first. Pick the next open item.
- **On completion:** delete the checklist line, write `docs/decisions/<prefix>/<prefix>-N.md`,
  prepend the index line to `DECISIONS.md`, update the summary table (per
  `.claude/rules/workflow-docs.md` in the dingine workspace — this repo is outside the
  `sync-workflow-surface` default target list, pass `-Targets` explicitly to receive the surface).

**Current status (2026-09-03):** Repo bootstrapped, no code yet. ANA-1 (data model + sync),
ANA-2 (orchestration design), ANA-3 (tooling & intelligence augmentations), ANA-4 (ACP protocol),
ANA-5 (execution model: granular control vs interactive), ANA-6 (OneDev necessity), and
ANA-7 (Infisical secret management) gate most MODs; MOD-1 (TUI scaffold) can start immediately.

---

## Open items

### Analyses

- [ ] **ANA-1 - Data model, box registry and sync topology.** Design the persistent layer before
  any storage code: Postgres schema (items, boxes, runs, sync state), the thin per-repo
  `items.json` shape (uuid, item key, title, status — nothing else, so the repo file stays
  merge-friendly and the DB carries the body), the OneDev issue mapping (item ↔ issue number,
  story/comments live in OneDev, sync over its REST API with a token — **no agent in the sync
  path**, it is a pure API client to keep token usage at zero), and the box registry (machine
  characteristics captured at registration: OS, arch, compiler + toolchain versions, env quirks —
  motivation: recurring build errors that differ per box; characteristics get injected into every
  agent prompt). Must settle: which store is canonical for which field (proposal: DB canonical for
  item state, OneDev canonical for story/discussion, `items.json` a derived snapshot for offline /
  fresh-clone use), conflict rule on concurrent edits from two boxes (proposal: `updated_at` +
  uuid, last-writer-wins per field, sync job reports divergence instead of silently merging), and
  where run transcripts live. Transcript proposal (settled 2026-08-30, session with maintainer):
  **split, two destinations** — the distilled step summary / final message goes to the OneDev
  issue as a *comment* (human story), the full raw transcript goes up as an issue *attachment*
  (big, on-demand debugging), written locally first and referenced as `run.transcript_ref text`
  (`file://...` until uploaded, `onedev://<issue>/<attachment>` after) with `prompt_digest` kept
  beside it; the uploader **must scrub secrets** (env vars, tokens echoed in tool output) before
  anything leaves the box, since an attachment is visible to everyone with repo access. Two more
  settled shapes the DDL must carry: **(a)** an `artifact` table (`item_id`, optional `run_id`,
  `kind` in `prd|plan|review|summary`, markdown `content` capped ~4-8 KB because it is
  prompt-injected, `created_at`) — agents never receive raw transcripts of previous items, the
  prompt builder injects linked items' artifacts instead (same index-plus-one-write-up economy the
  dingine workflow uses); `kind='summary'` is the DB-native decision write-up MOD-4 emits on
  completion. **(b)** an `item_link` edge table (`from_item`, `to_item`, `kind` in
  `blocks|origin|relates|supersedes`, composite PK) **replacing** any `blocked_on` array or
  `origin` column on `item` — traversal is a recursive CTE, no graph database; the prompt builder
  walks 1-2 hops and collects neighbors' `summary`/`review` artifacts, so htui bounds the token
  cost, not the agent. Output: `docs/ANA-1.md` with the schema DDL sketch. Spawns MOD-5, MOD-6,
  MOD-7.
- [ ] **ANA-2 - Orchestration pipeline design.** Design the multi-agent chain (ultracode-like, but
  agy is a first-class step): a run is a typed step graph, e.g. PRD (claude) → plan (claude) →
  implement (agy, possibly fan-out) → review (claude, cpp-reviewer persona for C++ targets), each
  step consuming the prior step's artifact. Must settle: step contract (input artifact, prompt
  template, output artifact, pass/fail gate), how a failed review loops back to implement, how the
  user approves between steps in the TUI, and the **skill-injection contract**: agents run with
  system skills disabled — htui owns the skill text and inlines it into the initial prompt
  directly, so behavior is identical on every box and the agent never spends turns reading skill
  files. Prompt-budget rule for every step: the initial prompt carries everything needed (item
  body, plan/PRD artifact, box characteristics from the registry, injected skill text, file
  excerpts chosen by htui) so the agent reads as few files as possible. Output: `docs/ANA-2.md`.
  Spawns MOD-4.
- [ ] **ANA-3 - Tooling and intelligence augmentations: Headroom, Serena, Graphify, and AST tools.**
  Evaluate and design the integration of external context optimizers, semantic code graphs, and
  LSP/AST intelligence tools into htui's agent execution and exploration layers:
  **(a) Headroom (Context Optimization & Token Compression):** Evaluate routing wrapped agent
  subprocesses (`claude`, `agy`) through a local Headroom proxy (`headroom proxy`, e.g. at
  `http://127.0.0.1:8787` via `ANTHROPIC_BASE_URL` or `headroom wrap`). Leverage reversible
  Compress-Cache-Retrieve (CCR), SmartCrusher (JSON), CodeCompressor (AST parsing), and CacheAligner
  (KV prefix cache stability) to compress large tool outputs and repetitively queried files by
  60–95%. Determine if htui manages the proxy daemon lifecycle or delegates to the host box
  registry (`ANA-1`/`MOD-7`). Also evaluate embedding Headroom's bundled tool binaries (`ast-grep` /
  `sg`, `difftastic` / `diff`, `scc` / `loc`) directly into the TUI Code Explorer and Diff tabs
  (`MOD-3`).
  **(b) Serena (LSP Semantic Code Intelligence via MCP):** Evaluate integrating Serena
  (`oraios/serena`) as a standardized MCP server across managed repos. Instead of agents performing
  fragile text surgery (regex/string replace) or reading entire files into context, Serena provides
  symbol-level navigation (`find_symbol`, `find_referencing_symbols`, `insert_after_symbol`, symbol
  renaming, type definitions) across 40+ languages (Rust, C++, Python, TS, etc.). Settle how htui
  dynamically configures Serena's LSP server per target repo and how htui can leverage Serena's
  symbol graph to auto-extract targeted code excerpts for `ANA-2`'s prompt budget contract.
  **(c) Graphify (Knowledge Graph, Architectural Hubs, & Pre-Tool Guards):** Evaluate integrating
  `graphify` graphs (`graphify-out/graph.json`): htui queries god-nodes (architectural hubs),
  community clusters, and 1–2 hop callflow relationships during initial prompt construction (`ANA-2`),
  giving agents instant architectural orientation without expensive exploration turns. Evaluate
  enforcing pre-tool hook guards (`graphify hook-guard`) to prevent agents from launching unconstrained
  grep/read spirals, exposing the `graphify.serve` MCP server, and aggregating multi-repo global
  graphs (`graphify global`) across htui-managed workspaces.
  **(d) Structural Diffing & Inspection:** Upgrade MOD-3's diff tab and review steps from raw line-based
  `git diff` to syntax-aware structural diffing (`difftastic` / `difft`), allowing agents and users to
  see semantic AST changes instead of whitespace or formatting noise.
  Must settle: whether htui acts as an MCP gateway/supervisor or writes ephemeral `.mcp.json` configs;
  how external tool availability is probed and recorded in the Box Registry (`ANA-1`/`MOD-7`); and
  fail-open vs fail-closed fallbacks when an external daemon (Headroom proxy, LSP server) is missing.
  Output: `docs/ANA-3.md`. Informs MOD-2, MOD-3, MOD-7.
- [ ] **ANA-4 - Agent communication protocol: ACP (Agent Client / Context Protocol) vs CLI subprocess streaming.**
  Investigate protocol-level integration standards between htui and underlying AI coding agents,
  focusing on ACP:
  **(a) Agent Client Protocol (Zed / open standard JSON-RPC 2.0):** Evaluate implementing an ACP
  client in Rust (`agent-client-protocol` crate) within htui instead of scraping or multiplexing ad-hoc
  CLI flags (`claude -p --output-format stream-json`, `agy ...`). ACP standardizes the boundary between
  the editor/harness and the agent: the agent acts as an ACP server emitting structured notifications
  (thought chunks, tool execution requests, proposed edits, permission checks), while htui acts as
  the ACP client managing tool approvals, workspace state, and diff presentations. Assess whether
  both target agents (`claude` / Claude Code, `agy` / Antigravity) natively support ACP or require thin
  adapters.
  **(b) Agent Context Protocol & Structured Memory Harnesses:** Evaluate standardized context-passing
  mechanisms (machine-readable task specifications, explicit boundary declarations) compared to htui's
  custom markdown `artifact` table and prompt templates.
  **(c) Trade-off Analysis:** ACP provides typed, bi-directional IPC, clean cancellation, structured
  tool interception, and client-driven diff reviews out of the box, avoiding brittle JSON stream
  parsing. Conversely, CLI subprocess wrapping requires zero external protocol support from agents
  that lack ACP servers. Settle: should htui adopt ACP as its primary internal agent driver trait
  (`MOD-2`), falling back to CLI streaming where ACP is unavailable?
  Output: `docs/ANA-4.md`. Informs MOD-2, MOD-4.
- [ ] **ANA-5 - Agent execution model: Granular per-message control with explicit context (@file) vs autonomous interactive sessions.**
  Evaluate and design the operational boundary between automated, step-by-step agent control and
  open-ended interactive user sessions during item execution:
  **(a) Granular Per-Message Control (Automated Step Engine):** htui acts as the strict conductor. For
  automated item execution (e.g. PRD, Plan, Implement), htui drives the agent turn-by-turn or
  message-by-message, explicitly including only relevant file context via `@file` references, AST
  excerpts, and prior step artifacts (`artifact` table). The agent is run with strict step bounds
  (single goal, verified outputs, immediate pass/fail gate) rather than an open-ended autonomous loop,
  eliminating runaway token burn, irrelevant file grepping, and divergent reasoning paths.
  **(b) Interactive Sessions (Exploration & Human Escalation):** Interactive sessions remain essential
  for initial brainstorming, user-steered refactoring, and deep debugging when automated steps fail
  their verification gates. In this mode, the agent runs with conversational agency in an interactive
  chat tab (`MOD-2`), receiving user follow-ups in real time.
  **(c) Seamless Handshake & State Transition:** Design the bidirectional bridge between both modes:
  automated steps run headless/turn-by-turn with `@file` injection and artifact capping; if a step fails
  or requests human clarification, htui seamlessly promotes the run into an interactive chat tab,
  preserving exact context, tool calls, and diff history. Once the human and agent resolve the blocker,
  htui extracts the step artifact, commits the decision summary, and resumes the automated pipeline.
  Must settle: syntax and resolution mechanics for `@` file/symbol inclusion in htui prompts; how per-turn
  token budgets are calculated; and how the TUI UI/UX handles transitioning between automated step cards
  and interactive streaming chat tabs.
  Output: `docs/ANA-5.md`. Informs MOD-1, MOD-2, MOD-4.
- [ ] **ANA-6 - Architectural necessity and alternatives to OneDev as issue tracker.** Evaluate
  whether an external OneDev instance is actually necessary in the htui topology, or whether it
  introduces excessive operational overhead and architectural bloat:
  **(a) Redundancy with Postgres:** htui's central Postgres database already holds item state,
  run history, machine characteristics, execution artifacts (`artifact` table), and relationship
  graphs (`item_link`). Evaluate storing human discussions, comments, and story descriptions directly
  in an `item_comment` table in Postgres, eliminating the need for an external sync service and
  token management.
  **(b) Operational & Infrastructure Weight:** OneDev is a heavyweight Java/JVM application requiring
  dedicated hosting, database management, and container maintenance. Evaluate the friction this imposes
  on single-developer or multi-box setups versus a zero-extra-infrastructure design where htui only
  depends on Postgres (or embedded SQLite for local/offline workflows).
  **(c) Alternatives for Discussion & Transcript Storage:** If OneDev is eliminated or made optional:
  how are long-form stories and transcripts stored? Options:
  1. *Pure Postgres + Object Storage:* Discussion in Postgres; full transcripts stored locally or
     in S3/MinIO / static HTTP file server.
  2. *Git-Native Storage:* Embed issue discussions and stories into repo-local markdown files or git notes
     (similar to git-bug / beads), keeping repos completely self-contained.
  3. *Pluggable Issue Tracker Trait:* Decouple issue tracking behind an optional `IssueSync` trait,
     making Postgres the sole canonical source of truth while supporting OneDev, GitHub Issues, or
     Linear as optional downstream sinks rather than hard architectural dependencies.
  **(d) Impact on Roadmap:** If OneDev is decoupled or dropped, assess simplifying ANA-1 (collapsing
  dual-canonical complexity to single-canonical Postgres) and eliminating or deferring MOD-5 (OneDev sync).
  Output: `docs/ANA-6.md`. Informs ANA-1, MOD-5, MOD-6.
- [ ] **ANA-7 - Secret management and injection: Infisical integration, multi-box sync, and transcript scrubbing.** Evaluate
  integrating Infisical as the centralized secret management layer across multi-box environments:
  **(a) Provisioning & Storage:** Centralized storage for Postgres credentials, OneDev API tokens,
  LLM API keys (Anthropic, Antigravity), and local proxy tokens. Evaluate Infisical Rust SDK vs.
  Infisical CLI (`infisical run -- ...`) for injecting environment variables into wrapped agent
  subprocesses (`claude`, `agy`) on demand.
  **(b) Multi-Box Synchronization:** How htui instances on different registered boxes (`MOD-7`)
  authenticate to Infisical (machine identities, universal auth, or local CLI session) without
  persisting plaintext secrets in repo files or local config databases.
  **(c) Transcript & Diff Scrubbing:** Leverage Infisical's secret inventory to dynamically generate
  high-precision regex/literal redaction masks, ensuring any secret managed by Infisical is
  guaranteed to be scrubbed from raw transcripts and diffs before they leave the box (`ANA-1`).
  **(d) Offline & Fallback Mechanics:** Fallback behavior when Infisical is unreachable (local encrypted
  cache via OS keyring / `.env.local` fallback).
  Must settle: Infisical SDK vs CLI subprocess wrapping; machine identity bootstrap flow per box;
  and secret masking pipeline for transcript uploads.
  Output: `docs/ANA-7.md`. Informs ANA-1, MOD-2, MOD-5, MOD-7.

### Next features

- [ ] **MOD-1 - TUI scaffold.** Rust binary, `ratatui` + `crossterm`, `tokio` runtime. Two main
  tabs: **Handoff items** and **Code explorer**. Handoff tab: left pane lists items with status
  badge; selecting one shows detail on the right (body, origin/blocked-on links, sync state, last
  runs) with a Run action. Tab bar supports dynamic tabs (agent chats, diff — MOD-2/MOD-3 add
  them). Reads items from `items.json` of a target repo passed on the CLI; DB wiring arrives with
  MOD-6, so keep the store behind a trait from day one.
- [ ] **MOD-2 - Agent session driver + chat tab.** Wrap `claude` and `agy` as subprocesses in
  headless/streaming mode (claude: `-p --output-format stream-json`; confirm agy's equivalent).
  Running an item opens a new tab streaming the chat (user/assistant/tool events rendered, input
  box for follow-ups). Prompt builder per the ANA-2 contract: one self-contained initial prompt
  (item body + box characteristics + injected skill text + htui-selected file excerpts + linked
  items' `summary`/`review` artifacts via a 1-2 hop `item_link` walk, per ANA-1 — never raw
  transcripts), system skills disabled on the agent invocation. Blocked on MOD-1; the prompt/skill
  contract is ANA-2's.
- [ ] **MOD-3 - Diff tab + code explorer.** Diff tab: working-tree diff of the target repo
  (`git diff` via `git2` or subprocess), side-by-side or unified, refreshable while an agent runs.
  Code explorer main tab: file tree + read-only viewer with syntax highlighting, enough to inspect
  what an agent touched without leaving htui. Blocked on MOD-1.
- [ ] **MOD-4 - Orchestrator** (from ANA-2). Implement the step graph: run state machine
  (pending/running/awaiting-approval/failed/done per step), per-step transcript persisted, review
  feedback looped back into an implement retry, per-step agent + model selection configurable per
  item or repo. Each step is a MOD-2 session under the hood; the TUI shows the chain's progress in
  the item detail pane. Blocked on ANA-2 and MOD-2.
- [ ] **MOD-5 - OneDev issue sync** (from ANA-1). Direct REST client against the OneDev instance
  (token auth), no agent involvement: create/update an issue per item, mirror status transitions,
  keep story + long-form discussion on the issue, pull remote edits back into the DB per ANA-1's
  conflict rule. Also carries the transcript path from ANA-1: post the step summary as an issue
  comment, upload the full transcript as an issue attachment (secret-scrub pass first), flip
  `run.transcript_ref` to the `onedev://` form. Manual "sync now" plus sync-on-item-change.
  Blocked on ANA-1 and MOD-6.
- [ ] **MOD-6 - Item store: `items.json` + Postgres** (from ANA-1). Implement the ANA-1 schema:
  `sqlx` (or `tokio-postgres`) migrations, the store trait from MOD-1 gets a DB-backed
  implementation, `items.json` written as a derived snapshot on change, uuid minted at item
  creation (this is what `handoff-add` becomes: htui creates the item row + snapshot line
  directly, no agent). Offline mode: read-only from `items.json` when DB is unreachable, queue
  writes. Blocked on ANA-1.
- [ ] **MOD-7 - Box registry + prompt injection** (from ANA-1). On first run per machine, register
  the box in the DB: hostname, OS + version, arch, compiler/toolchain inventory (probed:
  `gcc --version`, `cmake --version`, vcpkg presence, shells), free-form quirks field the user can
  edit. Every prompt built by MOD-2/MOD-4 includes the current box's characteristics so agents
  stop tripping on cross-box differences. `last_seen` updated per session. Blocked on ANA-1 and
  MOD-6 (DB), but the probe + a local cache can land with MOD-2 if sequencing demands it.

## Summary

| Area    | Open                                                                                     |
|---------|-------------------------------------------------------------------------------------------|
| ANA-N   | 7 (ANA-1 data model + sync, ANA-2 orchestration, ANA-3 tooling, ANA-4 ACP, ANA-5 exec model, ANA-6 OneDev necessity, ANA-7 Infisical secret management) |
| MOD-N   | 7 (MOD-1 TUI scaffold, MOD-2 agent driver, MOD-3 diff + explorer, MOD-4 orchestrator, MOD-5 OneDev sync, MOD-6 item store, MOD-7 box registry) |
| CLEAN-N | 0                                                                                         |
| TOOL-N  | 0                                                                                         |

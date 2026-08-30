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

**Current status (2026-08-30):** Repo bootstrapped, no code yet. ANA-1 (data model + sync) and
ANA-2 (orchestration design) gate most MODs; MOD-1 (TUI scaffold) can start immediately.

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
| ANA-N   | 2 (ANA-1 data model + sync, ANA-2 orchestration design)                                   |
| MOD-N   | 7 (MOD-1 TUI scaffold, MOD-2 agent driver, MOD-3 diff + explorer, MOD-4 orchestrator, MOD-5 OneDev sync, MOD-6 item store, MOD-7 box registry) |
| CLEAN-N | 0                                                                                         |
| TOOL-N  | 0                                                                                         |

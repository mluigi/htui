# CONCEPTS - htui

**Owner:** Luigi Marrandino
**Reviewed:** 2026-09-03

Standing architectural intent for `htui`. Governed by `.claude/rules/concept-docs.md`.

## What htui is

A cross-platform Rust TUI (`ratatui`, `crossterm`, `tokio`) that wraps AI coding agents (`claude`,
`agy`) to manage handoff workflows, step graphs, and execution across multiple repositories and
development machines.

It is **not** an IDE, not a code editor, not a general terminal multiplexer, and not a cloud-hosted
orchestration daemon. It is a local-first, developer-guided agent harness.

## Storage canonicity and sync topology

Storage is layered with strict single-canonical ownership per domain:

- **Postgres** is canonical for item state, box profiles, execution runs, and artifacts.
- **`items.json`** in managed repos is a derived, merge-friendly offline snapshot (UUID, key, title,
  status). It never holds task bodies or discussion.
- **Issue Tracker (OneDev / Git issues)** is canonical for human-facing story, comments, and
  discussion.
- **No agent in the sync path:** Synchronization between Postgres, local snapshots, and issue
  trackers is executed by pure API clients to keep token expenditure at zero.
- **Conflict law:** Divergence is detected via `updated_at` timestamps and UUIDs; concurrent edits
  report divergence instead of performing silent merges.

## Transcript privacy and the split model

Execution transcripts follow a split destination model:

- **Human narrative:** Distilled step summaries and final verdicts are posted as issue comments.
- **Debug data:** Raw, full transcripts are uploaded as issue attachments on demand.
- **Local scrubbing invariant:** All secrets, environment variables, and authentication tokens
  echoed in tool outputs must be scrubbed locally on the host box before any transcript or
  attachment leaves the machine.

## Prompt economy and skill injection

`htui` strictly bounds prompt token budgets through five invariants:

1. **Inlined skills:** Wrapped agents run with system skills disabled. `htui` owns the skill text and
   inlines it directly into the initial prompt so behavior is identical across boxes and agents never
   spend execution turns reading skill files.
2. **Bounded artifacts over raw history:** Agents never receive raw transcripts of prior items. The
   prompt builder injects only distilled artifacts (`summary`, `review`), capped at 4–8 KB.
3. **Recursive edge traversal:** Item relationships live in an `item_link` edge table traversed via
   recursive SQL CTEs (no graph database). The prompt builder walks at most 1–2 hops to collect
   neighbor artifacts.
4. **Host box awareness:** The host machine's hardware, OS, toolchains, and environment quirks are
   probed by the Box Registry and injected into prompts to eliminate cross-box build divergence.
5. **Targeted intelligence:** External context tools (Headroom compression, Serena LSP symbol graphs,
   Graphify hubs) provide symbol-level extracts rather than unconstrained file reading.

## Hybrid execution model

Execution separates automated progression from human exploration:

- **Automated Step Engine:** Multi-agent chains (PRD → Plan → Implement → Review) execute turn-by-turn
  with strict goal bounding, explicit file injection, and verification gates.
- **Interactive Session Promotion:** When a step fails, encounters ambiguity, or requires creative
  steering, `htui` promotes the session into an interactive streaming chat tab, preserving exact
  context without re-running prior steps.
- **Protocol decoupling:** The agent driver sits behind an abstraction trait, prioritizing ACP
  (Agent Client Protocol) for typed JSON-RPC streaming, with fallback to CLI subprocess streams.
- **Fail-open external tools:** Missing external daemons (Headroom proxy, Serena LSP, OneDev)
  degrade gracefully to local git commands and direct agent execution rather than blocking work.

## Which file answers which question

- `CONCEPTS.md` — standing invariants and why (the present).
- `HANDOFF.md` — outstanding work, active analyses, and live coordinates (the future).
- `DECISIONS.md` — reverse-chronological archive index of completed items (the past).
- `docs/decisions/<prefix>/<prefix>-N.md` — permanent write-up per completed item.
- `docs/ANA-N.md` — research, options, and arguments leading to an architectural verdict.
- Rules: `.claude/rules/concept-docs.md` and `.claude/rules/workflow-docs.md`.

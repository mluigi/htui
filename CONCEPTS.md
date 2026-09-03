# CONCEPTS - htui

**Owner:** Luigi Marrandino
**Reviewed:** 2026-09-03

Standing architectural intent for `htui`. Governed by `.claude/rules/concept-docs.md`. Distilled
from `docs/REQUIREMENTS.md`, which is the authority; requirement IDs in brackets.

## What htui is

A cross-platform Rust TUI (`ratatui`, `crossterm`, `tokio`) that wraps AI coding agents to run a
backlog of work items through configurable step graphs across projects, repositories and machines
[R-ID-1]. Single developer today, teams later without redesign [R-USR-1..3].

It is **not** an IDE, not a code editor, not a terminal multiplexer, and not a cloud service. It is
a local-first, developer-guided harness [R-ID-2].

## Single source of truth

- **Postgres holds everything**: items, documents, runs, full transcripts, skills, templates, box
  profiles, agent registry, settings [R-ID-3]. Credentials live in the OS keyring [R-STO-1].
- **Repos stay clean.** `htui` writes no files into managed repositories; only agents doing an
  item's work touch a working tree [R-ID-4]. Rejected: per-repo `items.json` snapshots, they were a
  second source of truth with merge exposure.
- **Offline is read-only** from a per-box cache; keys are minted online, so no offline collision
  and no sync engine [R-STO-3, R-STO-4, R-ENT-7].
- **No silent merges.** Concurrent edits on the same item version are rejected and shown as a
  divergence against the common ancestor; no timestamp last-writer-wins [R-ENT-10]. `version`
  guards the human-edited spec only; status moves are their own compare-and-set and never bump it
  (`docs/ANA-9.md` §4.2).
- **One cache writer.** The per-box cache is a SQLite mirror written only by the refresh task,
  never by user actions; live reads while connected go to Postgres (`docs/ANA-9.md` §6.3).
- **No agent in any bookkeeping path.** Sync, cache, import and status transitions are
  deterministic code [R-ID-6].

## Hierarchy

`workspace` groups projects for cross-repo work; `project` owns item keys, kinds, step graphs,
skill bindings and secrets; `repo` is a git checkout in exactly one project [R-ENT-1..4]. Workspaces
are explicit; a single project runs with none [R-ENT-2]. Logical identity never depends on a
filesystem path: every box maps repos and workspace roots to its own paths [R-BOX-4].

Item kinds are configurable per project with a key prefix and a default step graph; seeded set
`ANA`, `FEAT`, `FIX`, `CLEAN`, `TOOL` [R-ENT-6]. Item links are UUID edges, so cross-project
dependencies need nothing extra [R-ENT-9].

## Agents and orchestration

- **One driver trait, ACP first.** Agents speak Agent Client Protocol; a CLI stream adapter covers
  the rest. Adding an agent is a registry row plus at most a parser [R-AGT-1..5]. Version one:
  `claude`, `agy`.
- **Step graphs, not scripts.** A run is an ordered list of phases with candidate agents, gates,
  retries, fan-out and isolation mode; review failure loops back to implement [R-ORCH-1..3, 7, 8].
- **Manual and auto.** Manual runs one item with gates as configured; auto is a queue runner that
  downgrades non-hard gates and honors caps [R-ORCH-4, 6]. Any step can be promoted to interactive
  chat and resumed [R-ORCH-5].
- **Capability matching.** Items declare required tags, boxes declare and probe theirs; a mismatch
  refuses the run [R-BOX-3, R-ORCH-10].
- **Quota-aware routing.** Candidate agents are tried in priority order, skipping exhausted
  subscriptions or reached token caps [R-AGT-7, 8].

## Prompt economy

- **Inlined skills.** Agents run with system skills disabled; `htui` owns skill and template text
  in Postgres and inlines it into one self-contained initial prompt [R-ID-5, R-PRM-1, R-SKL-1, 2].
- **Bounded context.** Prior work reaches a prompt only as documents and upstream summaries within
  one to two hops inside the active workspace, never as raw transcripts; oversize inputs are trimmed
  in a fixed priority order [R-PRM-1..3].
- **htui MCP server.** Agents propose links, write artifacts and, when exposed, queue heavy
  commands behind per-box limits through `htui`'s own MCP tools, scoped to their run step
  [R-MCP-1..4].

## Secrets

Project secrets come from a `SecretProvider` (Infisical first) into the agent environment only;
agents never get a tool that reads them [R-SEC-1, 2]. Every transcript is scrubbed on the host box
before it persists, and scrubbing fails closed [R-ID-7, R-SEC-3].

## Which file answers which question

- `docs/REQUIREMENTS.md` - what the product must do, with stable IDs (the contract).
- `CONCEPTS.md` - standing invariants and why (the present).
- `HANDOFF.md` - outstanding work and live coordinates (the future).
- `DECISIONS.md` - reverse-chronological index of completed items (the past).
- `docs/decisions/<prefix>/<prefix>-N.md` - write-up per completed item.
- `docs/ANA-N.md` - research and verdict per analysis. ANA-1 and ANA-8 are superseded by
  `docs/REQUIREMENTS.md` §15 where they conflict.

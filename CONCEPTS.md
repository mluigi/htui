# CONCEPTS - htui

**Owner:** Luigi Marrandino
**Reviewed:** 2026-10-20

Standing architectural intent for `htui`. Governed by `.claude/rules/concept-docs.md`. Distilled
from `docs/REQUIREMENTS.md`, which is the authority; requirement IDs in brackets.

## What htui is

A cross-platform Rust TUI (`ratatui`, `crossterm`, `tokio`) that wraps AI agents to run a backlog
of work items through configurable step graphs across projects, repositories and machines. Projects
of any kind, coding the main one [R-ID-1]. Single developer today, teams later without redesign
[R-USR-1..3].

A local-first, developer-guided harness. It ships no editor of its own; it hosts your `$EDITOR`. A web
front end and multiplexer panes are later tier. A headless `htui worker` per box, started by you and
talking only to your Postgres, is part of `htui` [R-ID-2].

## Single source of truth

- **Postgres holds everything**: items, documents, runs, full transcripts, skills, templates, box
  profiles, agent registry, key bindings, settings [R-ID-3]. Credentials live in the OS keyring; a
  headless worker without one reads its DSN from a systemd credential or stdin, dev and CI from an
  env var stripped from agents; never argv or a plain file [R-STO-1].
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

`workspace` groups related projects and owns cross-project items [R-ENT-16]; `project` owns item
keys, kinds and secrets, and so does a workspace, for its own work [R-ENT-3]; a step graph is owned by a project,
a workspace or the global workflow library [R-ORCH-1]; skills attach from global down to phase, most specific wins
[R-SKL-2]; a `repo` is a git checkout or plain folder in one project, zero or more per project
[R-ENT-1..4]. Workspaces are explicit and not a directory their projects must live under; each has a
folder for what `htui` generates there (compose file, environment file); a single project runs with
none [R-ENT-2, R-ENT-17, R-SEC-5].
Logical identity never depends on a filesystem path: every box maps repos to its own paths [R-BOX-4].

Item kinds are configurable per project, seeded by its type [R-ENT-6]. Item status is open, blocked
or closed; progress lives on runs, and what a person sees is derived from the latest run [R-ENT-8]. Links are UUID edges, so cross-project dependencies need
nothing extra [R-ENT-9].

## Agents and orchestration

- **One driver trait, ACP first.** Agents speak Agent Client Protocol; a CLI stream adapter covers
  the rest. Adding an agent is a registry row plus at most a parser [R-AGT-1..5]. Version one:
  `claude`, `claude-cli`, `agy`.
- **Step graphs, not scripts.** Phases with candidate agents, gates, retries, loop-backs and
  isolation; fan-out divides work into parallel parts that an integrate step merges [R-ORCH-1..3, 7,
  8]. A coordinator per project and workspace only manages [R-ORCH-14]. Global workflow templates,
  loops included, are picked per run by a light model or pinned per item, and runs unroll them
  [R-ORCH-1].
- **Runs go where you say.** Each item runs on a target — this box, docker, a remote machine over
  SSH or its docker — chosen by a Settings → workspace → project → workflow → item cascade; `htui`
  prepares it there and the work comes back by collecting the branch [R-ORCH-15]. Stopping a run
  saves its conversation and can always be resumed [R-ORCH-16]; leaving the TUI never stops work
  [R-TUI-18].
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
  one to two hops inside the active workspace, and as capped memory (global memory the one input from
  outside it), never as raw transcripts; template and skills are never trimmed, the rest in a fixed
  order [R-PRM-1..3].
- **Memory.** Verbatim facts, decisions, gotchas and lessons of a project, workspace or the global
  scope, written by the light model with no person's gate; checks, freshness and selection are code
  [R-ENT-13, R-ID-6].
- **htui MCP server.** Agents propose links, write artifacts and, when exposed, queue heavy
  commands behind per-box limits through `htui`'s own MCP tools, scoped to their run step
  [R-MCP-1..4].

## Secrets

Project secrets come from a `SecretProvider` (Infisical first) into the agent environment only;
agents never get a tool that reads them [R-SEC-1, 2]. Everything persisted or sent off the box is
scrubbed on the host box first, and scrubbing fails closed [R-ID-7, R-SEC-3].

## Which file answers which question

- `docs/REQUIREMENTS.md` - what the product must do, with stable IDs (the contract).
- `CONCEPTS.md` - standing invariants and why (the present).
- `HANDOFF.md` - outstanding work and live coordinates (the future).
- `DECISIONS.md` - reverse-chronological index of completed items (the past).
- `docs/decisions/<prefix>/<prefix>-N.md` - write-up per completed item.
- `docs/ANA-N.md` - research and verdict per analysis. ANA-1 and ANA-8 are superseded by
  `docs/REQUIREMENTS.md` §15 where they conflict.

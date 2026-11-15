# MOD-127 - Generic Toolbox binding; MCP server, command and env-preset kinds

Origin: ANA-40 (`docs/ANA-40.md` §4.3, §8 P2 and P3; verdict `docs/decisions/ana/ana-40.md`, all nine
recommendations accepted 2026-10-17). Requirements: `R-SKL-2`, `R-TUI-7`, `R-ID-3`, `R-ID-5`,
`R-AGT-11`; touches `R-SEC-2`, `R-ID-4`, `R-AGT-5`. Item: `HANDOFF.md`, MOD-127.

## Problem
A person running work through htui can control exactly one kind of run resource: skills. Skills attach
at global, project and phase level, while the requirement (`R-SKL-2`) already asks for workspace and
item-kind levels as well. Everything else a run could use has no home. htui's own MCP server is a
per-session lease, a user cannot add another MCP server, reusable slash-style commands do not exist,
and named environment sets do not exist. Leaving it unsolved keeps the Toolbox (`R-TUI-7`) a skills
list, blocks bundles (MOD-128) and hooks, sub-agents and mods (MOD-129), and sends people back to
ambient agent config, which `R-ID-5` forbids.

## Evidence
- ANA-40 §2.2 code survey (2026-10-17): the only attach model in code is skills (three levels) plus
  persona-per-phase; MCP servers have "no table, no user-configured servers"; commands, env presets
  and the workspace and item-kind levels "do not exist".
- Current code: `SkillLevel` has `Global`, `Project` and `Phase`; a skill binding row keys on skill,
  project and phase only.
- `R-SKL-2` (amended 2026-10-15) and MOD-113 (2) require the workspace and item-kind levels; neither is
  built.
- The maintainer accepted the generalised binding as a decision (ANA-40 §9 call 4) and filed this item
  to build it.
- No user quotes or measurements exist. Whether people want the three kinds beyond Gortex, Headroom,
  Serena and Graphify (MOD-128) is an assumption — needs validation via use once MOD-124 shows them.

## Users
- **Primary**: the person who runs a backlog across several projects (the maintainer today). They set
  up an MCP server, a reusable command or an environment once, attach it where it belongs (a whole
  workspace, one project, one item kind such as "docs", or one phase) and expect each run to start
  with exactly that set and be able to see why.
- **Secondary**: the global htui agent (`R-AGT-11`), which adds Toolbox entries without asking;
  MOD-124, MOD-128 and MOD-129, which build on this binding.
- **Not for**: agents other than Claude CLI and ACP agents (marked unsupported, not rendered);
  hooks, sub-agents and mods (MOD-129); bundles and probe status (MOD-128); the Toolbox tab shell and
  workflow editor (MOD-124).

## Hypothesis
We believe **one typed binding, with one resolver shared by both stores, plus MCP server, command and
env-preset entries** will **let a person attach run resources at the level they belong to and get an
exact, explained set per run** for **people running htui across projects**.
We'll know we're right when **every skill attachment resolves exactly as before, a run launched with
a bound MCP server, command and env preset receives precisely the resolved set on the Claude CLI and
ACP paths, and the run's step record names, for each, the level it came from or the reason it was
skipped**.

## Success Metrics
| Metric | Target | How measured |
|---|---|---|
| Skill behaviour preserved | 100% of existing skill attachments resolve to the same body and version after the change | Golden resolver cases built from the pre-change rules, run on both stores |
| Store parity | Postgres and in-memory resolvers agree on every case | One shared conformance suite for both |
| Exact frozen set | A launched run holds exactly the resolved set, nothing ambient | Launch tests on Claude CLI and ACP: rendered config equals the resolved set, no stray server |
| Skips are explained | 100% of resources a run's agent cannot use are skipped with a recorded reason, and the run still starts | Unsupported-agent test per kind; the step record names each skip |
| Secrets stay out | 0 secret values in any stored record, log, `Debug` output or file in a managed repo | Scrubber and file-location tests (`R-SEC-2`, `R-ID-4`) |
| Levels | Workspace and item-kind levels work for skills and for each new kind | Most-specific-wins cases across all five levels |

## Scope
**MVP**
- One typed binding replacing the skill-only binding: kind, reference, level (global, workspace,
  project, item kind, phase), pin or follow latest, activation (`always`, `glob`, `off`). Most specific
  level wins per resource; existing skill attachments keep their meaning.
- One resolver shared by the Postgres and in-memory stores, and a step-level reason record in the
  trim-record style skills use, now listing every kind attached.
- Workspace and item-kind levels for skills (closing the part of MOD-113 (2) that is about levels).
- Three new kinds, each a Postgres record with edit-as-form in the Toolbox surface:
  - **MCP server**: a neutral record (name, transport, command and arguments or URL and headers,
    environment, tool allow and deny), rendered per agent at launch for the Claude CLI and ACP
    paths; other agents marked unsupported.
  - **Command**: a reusable prompt body delivered to the agent in a way each supported path allows.
  - **Env preset**: a named key and value set, secret values by reference only, merged by precedence.
- A run receives an exact frozen set; a kind the agent cannot use is skipped and the skip recorded.
- The global htui agent can add entries of these kinds without asking; edits and deletions ask
  (`R-AGT-11`).
- Entries rendered only at run launch, to an htui-owned location or by launch arguments, never into a
  managed repository (`R-ID-4`).

**Out of scope**
- Hooks, sub-agents, mods and their per-agent capability flags — MOD-129.
- Bundles, linked parts, probe install, daemon and proxy status — MOD-128.
- The Toolbox tab shell, grouping of every kind, and the workflow editor — MOD-124. Forms for the new
  kinds land beside the existing Skills views and MOD-124 regroups them.
- Renderers for agents other than the Claude CLI and ACP paths — recorded as unsupported.
- A `required` flag that turns a skip into a run failure — ANA-40 mentions it; this item's text does
  not ask for it. Revisit if a person needs it.
- Importing Claude Code plugins into parts (MOD-129 with MOD-113 (4)); skill resources and template
  library (rest of MOD-113).
- Edits to `docs/REQUIREMENTS.md` and `CONCEPTS.md` (maintainer-gated; ANA-40 lists them as owed).

## Delivery Milestones
<!-- Business outcomes, not engineering tasks. /plan turns each into a plan. -->
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Typed binding and resolver | A person can attach a skill at global, workspace, project, item-kind or phase level; one shared resolver gives the same answer on both stores; each step records what it received and why. Every earlier skill attachment behaves as before. | complete | `.claude/plans/mod-127-toolbox-binding-m1.plan.md` |
| 2 | MCP server kind | A person can define an MCP server, attach it at any level, keep its secrets by reference, and have runs on Claude CLI and ACP start with exactly the resolved servers; other agents skip it with the reason shown. | in-progress | `.claude/plans/mod-127-toolbox-binding-m2.plan.md` |
| 3 | Command and env-preset kinds | A person can define reusable commands and named environment sets, attach them like any kind, and see them reach the run (or be skipped with a reason); the htui agent can add them unasked. | pending | — |

## Open Questions
Each carries the default the plan takes; the run does not stop for them. A different answer from the
maintainer amends the plan before implementation.
- [ ] **Where do the new forms live until MOD-124?** Default: new views beside Skills and Templates in
  the existing Skills tab, moved by MOD-124 into Toolbox groups. Alternative: wait for MOD-124 and
  ship the data and run path only.
- [ ] **How does a command reach a run under `R-ID-5`?** Default: inlined into the prompt through the
  existing placeholder path, which works on every agent. Alternative: also written as a slash command
  where the agent supports it, which needs a per-run materialisation path.
- [ ] **What is a "secret by reference" for an MCP server or env preset?** Default: the name of a
  project secret that `R-SEC-2`'s run-start resolution already supplies, resolved only into the agent
  subprocess environment, never stored. Alternative: a keyring reference of its own.
- [ ] **How does an env preset combine across levels?** Default (ANA-40 §4.3): presets merge, with
  target, project and step precedence, unlike skills and MCP servers where the most specific entry
  wins whole. Confirm that item-kind and workspace joins this precedence.
- [ ] **Does an MCP server override replace or merge?** Default (ANA-40 §2.4): the most specific
  entry per server name wins whole, no field merge.
- [ ] **Does a migration of existing `skill_binding` rows need a window of dual reads?** TBD — needs
  validation via the plan's fact-check of the store and the upgrade path.

## Risks
| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Generalising the skill resolver changes behaviour that Pg and in-memory stores both rely on | Medium | High | Golden cases from the old rules written first; conformance suite over both stores |
| A new migration moves count pins and snapshots in unrelated tests | High | Low | Plan lists every pin; new-migration checklist from earlier items |
| The MCP and command renderers diverge per agent and grow | Medium | Medium | Claude CLI and ACP only; others unsupported by data, not by branch |
| A secret value leaks through a rendered MCP file or a record | Low | High | Secrets by reference; scrubber tests; no file in a managed repository |
| Overlap with MOD-113 (2) and MOD-124 duplicates work or collides in the Skills tab | Medium | Medium | This item owns the levels and kinds; MOD-113 keeps template library, resources and import; MOD-124 owns the shell |
| MOD-126 and MOD-130 land migrations at the same time | Medium | Low | Migration numbers are taken at merge; plan names no number |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*

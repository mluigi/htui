# ANA-40 - Toolbox: kinds of run tools and how to show them (concluded, 2026-10-17)

Full analysis: `docs/ANA-40.md` (prior art, options, sources, what was not verified). Origin: the UI
redesign review, `docs/ui-redesign-findings-2026-10-15.md` §4.6. Addresses `R-TUI-7`, `R-ORCH-1`,
`R-PRM-4`, `R-SKL-1`, `R-SKL-2`, `R-SKL-4`, `R-LATER-7`, `R-ID-6`.

## Verdict

The maintainer accepted all nine recommendations of `docs/ANA-40.md` §9 ("all as recommended").

- **Kinds.** Eleven kinds plus bundles as a twelfth entry type: workflows, skills, personas,
  templates, mods, MCP servers, hooks, commands, sub-agents, prompts, env presets, tools (bundles).
  "Agent help" leaves the Toolbox for Settings > Agents.
- **Storage.** Postgres is the source of truth (desired state) for every kind; the box probe reports
  observed state and never writes the Toolbox without a person, or the htui agent, accepting it.
  Files are written only at run launch, outside managed repositories (`R-ID-4`).
- **Bundles.** A bundle (Gortex, Headroom, Serena, Graphify) is one `tools` entry with linked parts,
  each toggleable alone; default-on for coding projects (`R-LATER-7`), switchable off per project.
- **Attachment.** Every attachable kind reuses `R-SKL-2`'s levels through one generalised binding
  (the skill binding made typed). A run receives an exact frozen set; a kind the agent cannot use is
  skipped and the skip recorded.
- **htui agent.** May add Toolbox entries unasked (`R-AGT-11`); edits and deletions ask.
- **Workflows.** A workflow is the authored global template; ANA-33 owns the unrolled run graph. A
  kind's default step graph is treated as a degenerate workflow (only `next` links); migrating
  `step_graph` into the workflow model is ANA-33's call, not filed here.
- **Light model.** One `app_setting` key `{agent_id, model}` (ANA-30 option B, a partial reversal of
  its "no `utility_model` setting"). The run-start pick is advisory, recorded as a `pick workflow`
  step, falls back to the Settings default workflow and then the kind's default graph, and needs no
  acceptance in auto mode.
- **Requirement edits owed** (maintainer-gated, `docs/REQUIREMENTS.md` is not edited at close-out):
  define each kind's attachment in `R-TUI-7`; add the global column to its skill matrix; drop "agent
  help" from the findings doc; amend `R-PRM-1`'s inventory; refresh `CONCEPTS.md` (it omits the
  Toolbox and the light model).

## Spawned items

The doc proposed seven items (P1-P7); four were filed, merged by dependency:

| Filed | From | Subject |
|---|---|---|
| MOD-126 | P1 | Light model setting and run-start workflow pick |
| MOD-127 | P2 + P3 | Generic Toolbox binding; MCP server, command and env-preset kinds |
| MOD-128 | P4 | Bundle entries with linked parts and probe status |
| MOD-129 | P5 + P6 | Hooks, sub-agents and mods as run resources; plugin import into parts |

P7 (kind default graphs as workflows) is carried into ANA-33's verdict. MOD-124 (Toolbox tab and
workflow editor) stays as filed and now builds on this verdict.

## Not verified

See `docs/ANA-40.md` §10: R-AGT-5, R-HIS-1 and R-SKL-3 text; ANA-39's design; whether per-run mod
loading works under isolation (a spike inside MOD-129); whether the box probe extends past
`kind:path` tools; web claims that were recalled, not fetched.

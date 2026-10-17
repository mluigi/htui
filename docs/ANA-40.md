# ANA-40 - The Toolbox: kinds, storage, attachment, workflows, light model

> **Scope note:** HANDOFF item ANA-40 (`HANDOFF.md:184-198`): the Skills tab becomes a Toolbox of everything a run can use. The ANA settles (1) which kinds exist and which are Postgres data versus discovered on a box, (2) how each kind is shown and edited (a bundle as one entry or as its parts), (3) how each kind attaches to a run, and (4) what workflows are against ANA-33's step graphs, and the light-model setting that partly reverses ANA-30. Governed by `.claude/rules/workflow-docs.md`, `CONCEPTS.md`, `docs/REQUIREMENTS.md`, ANA-30, ANA-33 (unwritten, see 2.6), ANA-39, ANA-41, ANA-42. Origin: HANDOFF item ANA-40 (`HANDOFF.md:184-198`), added 2026-10-17 in 8ed3bb07. Reference design: `docs/ui-redesign-findings-2026-10-15.md` (F) section 4.6 and the throwaway skeleton on `spike/ui-skeleton`.

> **Requirements addressed:** R-TUI-7, R-ORCH-1, R-PRM-4, R-SKL-1, R-SKL-2, R-SKL-4, R-LATER-7, R-ID-6.
> Touched at their seams: R-ID-3 (Postgres is the source of truth), R-ID-4 (no files written into managed repositories), R-ID-5 (agents run with their own system skills off), R-PRM-1 (prompt inventory), R-AGT-8 (personas carry no agent), R-AGT-10 (tool-manager install routes), R-AGT-11 (htui agent may add Toolbox entries without asking), R-ORCH-15 (target pin on a workflow), R-TUI-8 (kinds' step graphs live in Settings), R-TUI-17 (rewrite uses the light model), R-BOX-2 (box probe).

> **Status (2026-10-17): accepted.** The maintainer accepted every recommendation in section 9 as written ("all as recommended"). No implementation code is written here. Verdict in section 7. Proposed MOD items in section 8 are described only; none is minted or filed. Close-out path on acceptance: `docs/decisions/ana/ana-40.md`.

---

## 1. Context and problem statement

The 2026-10-15 UI review (F) renamed the Skills tab to Toolbox (F:150) and said it must hold "every kind of tool a prompt item's execution can use" (F:374). R-TUI-7 now lists the kinds (REQUIREMENTS.md:580-587) but leaves their data model open: "ANA-40 settles the kinds and how each is shown." F makes no decision on Postgres versus discovered, bundle presentation, or attachment (F:427-428: no existing item covers a toolbox of all kinds, the per-item workflow pick, or mods as a run resource).

Four questions:

1. Which kinds exist; which are Postgres data, which are discovered on a box.
2. How each is shown and edited; is a bundle one entry or its parts.
3. How each attaches to a run.
4. What a workflow is against ANA-33's step graphs; what the light-model setting does to ANA-30.

## 2. Surface as read

### 2.1 What the skeleton shows (F:377-381; skeleton `data.rs`, `app.rs`)

Groups in order: workflows, skills, personas, templates, mods (claude only), MCP servers, agent help, hooks, commands, sub-agents, prompts (rewrite-item, fast-pick), env presets, tools (gortex, headroom, serena, graphify). Everything except workflows is one flat mock struct `Skill { name, group, desc, attach, body }` with `attach` as free text. The `e edit` and `t attach` hints have no handlers. Only workflows are structured (steps, typed links, version, optional target pin). The skeleton therefore defers the data-model question to this ANA; it is a presentation sketch, not a schema.

### 2.2 What exists in code today (code survey, paths relative to repo root)

| Kind | Today |
|---|---|
| Skills | Postgres: `skill`, `skill_version`, `skill_binding` (0001_init.sql:406-432, amended 0007). Levels Global < Project < Phase (`model/skill.rs:18-120`); most specific wins (`resolve`, skill.rs ~312); inlined via `{{skills}}` (`prompt/template.rs:97`). Import from SKILL.md and rules dirs (`skill_import.rs`). |
| Personas | Postgres `persona` (0012_persona.sql), one per phase (`step_graph_phase.persona_id`), frozen into `run.graph_snapshot`. Body renders as a protected section; `tools` and `permission` only narrow the agent's (`htui-agent/src/driver.rs:212-235`). Never names an agent (persona.rs:1-8). Imported from `.claude/agents/*.md`. |
| Templates | Postgres `prompt_template`, per project, versioned (0001:266). R-PRM-4 now says a shared library with per-project overrides; not yet implemented. |
| Mods | None as a registry. htui's own plugin is written by `htui claude emit` (`claude_emit.rs`). User extensions are isolated out of runs (`ISOLATION_ARGS`, `htui-agent/src/cli/mod.rs` ~258-275); per-row opt-out `settings.user_extensions`. |
| MCP servers | Only htui's own, a per-session runtime lease (`McpServerSpec`, driver.rs:244; `cli/mcp_file.rs`). No table, no user-configured servers. |
| Hooks | No kind; Claude hooks disabled in runs (`disableAllHooks`). |
| Commands | Two unrelated things: htui's `command_run` queue (R-MCP-3) and Claude slash commands (disabled in runs). |
| Sub-agents | No kind; only observed (`subagent_spawned` stored as `other`, acp/map.rs). `.claude/agents` files import as personas. |
| Prompts (rewrite, fast pick), env presets, light model | Do not exist in code. |
| Bundle tools | Do not exist; only agent-adapter install exists (`htui-agent/src/install/`, R-AGT-10). |
| Discovered on a box | `box_tool`, `agent_box`, `box.probed_tags`; spec `box_probe_spec` overlaid via `app_setting`, `kind:path` tools only (`box_probe/spec.rs`). |

Fact: the only attach model in code is skills (R-SKL-2) plus persona-per-phase. Nothing is named "Toolbox" in code; the tab is `SkillsTab` with a `Skills | Templates` switch (`ui/tabs/skills/mod.rs:1-60`).

### 2.3 What the requirements say

- R-TUI-7 lists the kinds; R-SKL-2 attachment levels (global, workspace, project, item-kind, phase; most specific wins; pin or follow latest; `always|glob|off`) are defined only for skills.
- R-ID-3/CONCEPTS.md:35: Postgres holds everything. R-ID-4: no files in managed repos. R-ID-5: agents run isolated; htui owns prompt and skill text.
- R-ORCH-1 workflow paragraph: global library; typed links `next`, `approved`, `asks you`, `if`, loops `changes requested`, `fails`, `retries`; a step names a persona and a specific or any agent; a light model picks unless the item pins; run unrolls; agent may change it at run time per a setting.
- R-ID-6 (amended 2026-10-17): the light model is outside the no-LLM-in-bookkeeping rule; one model serves both uses, "partly reverses ANA-30's rejection of a single `utility_model` setting."
- R-AGT-11: htui agent adds Toolbox entries without asking; deletes and target changes always ask.
- R-LATER-7: Gortex, Serena, Graphify, Headroom mandatory in coding sessions; missing ones are installed via R-AGT-10's routes or the user is told how.

### 2.4 Web prior art (fetched unless marked recalled; summaries are small-model, so treat as indicative)

- Bundles are never one artifact. Gortex: machine setup, daemon, per-repo track, per-repo init (MCP, hooks, instructions). Headroom: a local proxy plus env or base-URL change, or an MCP server. Serena: an MCP command with layered global/project config and per-client "contexts" that disable overlapping tools. Graphify: a skill, a PreToolUse hook, a git hook, a repo artifact; MCP is a separate opt-in (do one route, not both).
- Claude Code plugins are one install unit with components (skills, commands, agents, hooks, hooks module = "mod", MCP, monitors); `/plugin` shows one row expandable to components. Scopes user / project / local. MCP precedence local > project > user > plugin; whole-entry winner, no merge. `--strict-mcp-config` and `--setting-sources` let a run force an exact set.
- The same MCP record serialises into four incompatible shapes (Claude JSON `mcpServers` with `type`; Codex TOML `[mcp_servers.*]`; Gemini JSON with `httpUrl`; Zed `context_servers`). Env expansion syntax differs.
- Cursor groups by attach mode (always, glob, agent-requested, manual). Continue has model roles (chat vs autocomplete), an analogue of the light model. Unverified/recalled: Cursor `mcp.json` paths, Continue Hub, Serena exact launch command.

### 2.5 Conflicts among the sweep notes, resolved

1. **"agent help".** F lists it as a Toolbox group and the skeleton seeds it (claude, codex docs, attach "-"); ANA-40 (HANDOFF:187-189) and R-TUI-7 do not. It is never defined. Resolution: drop it from the Toolbox. Documentation on how htui drives an agent is agent-driver code, not a run resource. If wanted, it belongs to the agent row in Settings > Agents. Flagged for the maintainer (call 1).
2. **Skill levels.** CONCEPTS.md:47-51 lists global to phase; R-SKL-2 has five levels (workspace and item-kind added 2026-10-15); code has three (Global, Project, Phase). Resolution: R-SKL-2 is normative; code and CONCEPTS.md lag (MOD-113 territory). R-TUI-7's matrix omits a global column; treat that as a requirement typo (call 8).
3. **"prompts" vs "templates".** R-TUI-7 lists both; R-PRM-4 defines templates as per-phase prompt templates. Resolution: templates are phase/item-body templates (the prompt-assembly input); "prompts" is exactly the two light-model prompts (rewrite, fast pick) per F. Different consumers, different attach points.
4. **"commands".** Slash-command definitions (a Claude-style reusable prompt) versus htui's `command_run` queue. Resolution: the Toolbox kind is the former; `command_run` stays a phase property (R-MCP-3) and is not a Toolbox kind.
5. **Light model: item work or draft?** R-ID-6 classes the workflow pick as item work and the rewrite as a person-accepted draft, while R-ORCH-1 lets the pick apply without acceptance. Resolution: consistent, because the pick is advisory, pinnable per item, recorded, and no bookkeeping path reads it; the deterministic fallback in section 5.4 covers failure. Not a conflict once the pick is recorded (R-HIS-1 is the assumed home; unverified).
6. **Postgres everywhere versus box-resident kinds.** CONCEPTS.md says Postgres holds everything; Claude-side kinds live in files. Resolution in 5.1: Postgres holds desired state; the box holds observed state.
7. **Mandatory bundles versus optional attachment** (R-LATER-7 versus a Toolbox of per-run attachments). Resolution in 5.3: bundle attachment is a default-on binding that a project can switch off.

### 2.6 ANA-33 and ANA-34 do not exist

`docs/ANA-33.md` and `docs/ANA-34.md` are unwritten HANDOFF items (`HANDOFF.md:111-139`). "What ANA-33 settled" is nothing yet; only its brief exists: graphs that branch, loop, run phases in parallel, are workspace-owned, can be planned at run time within project bounds; fan-out becomes division of work with an integrate step. This ANA settles the workflow side so ANA-33 can build on it, and says so in section 5.5.

## 3. Prior art inside htui

- Skill library with versions, bindings and a resolution function shared by Pg and mem backends (`htui-store/src/pg/read.rs:1610`): the model to copy for other attachable kinds.
- Persona freeze into `graph_snapshot` (htui-orch `graph.rs:414-426`): the model for run snapshot semantics ("runs already going keep the version they started with", F:407).
- Box probe with an editable overlay (`box_probe_spec`): the model for "discovered on a box".
- Agent install pipeline (`install/`, consent before download): the model for R-LATER-7 install (ANA-39 owns it).
- ANA-30 option B (one `app_setting` key `{agent_id, model}`, built inside the first MOD that adds a second drafting surface) is the held-in-reserve design for the light model.

## 4. Options per sub-question

### 4.1 Which kinds exist, and where they live

| | A. Everything in Postgres | B. Postgres desired state plus box observed state | C. Discovery only for box-resident kinds |
|---|---|---|---|
| Shape | One table family per kind | Neutral record in Postgres; probe reports what a box actually has | Read the box's files; no htui record |
| R-ID-3 | Fits | Fits | Breaks for mods, MCP, hooks |
| R-ID-4/5 | Needs materialisation | Materialise per run into an htui-owned path or launch argument | Agent loads ambient files, against R-ID-5 |
| Portability across agents | Good (per-agent renderers) | Good | Poor |
| Failure | Row says installed, box lacks it | Visible mismatch shown as status | Nothing to attach reliably |

Recommended: B. Per kind:

| Kind | Source of truth | Notes |
|---|---|---|
| workflows | Postgres, global, versioned | F:422; project workflows later (ANA-41) |
| skills | Postgres (R-SKL-1) | unchanged |
| personas | Postgres (exists) | unchanged |
| templates | Postgres, shared library plus per-project override (R-PRM-4) | |
| prompts (rewrite, fast pick) | Postgres, versioned, two fixed slots | not user-addable beyond editing |
| mods | Postgres record (name, source, version) plus box state | claude only; see 4.3 |
| MCP servers | Postgres neutral record `{name, transport, command/args/env or url/headers, tool allow/deny}` | rendered per agent at launch (Claude `--mcp-config` file, ACP `session/new`, others by renderer) |
| hooks | Postgres record | claude-family only today; see 4.3 |
| commands | Postgres (markdown body, like a skill) | inlined or rendered as a slash command where the agent supports it |
| sub-agents | Postgres record, distinct from personas | a persona is a step posture; a sub-agent is a callable delegate |
| env presets | Postgres: named key/value set; secret values by keyring reference only (R-SEC-2) | new |
| bundle tools | Postgres record naming its parts, plus box probe for binary/daemon presence | see 4.2 |

"Discovered on a box" is only observation: binary presence and version (extend `box_probe_spec` beyond `kind:path`, unverified feasibility), daemon/proxy status, and, for Claude-side kinds, what the agent itself has configured. Discovery never becomes the source of truth.

### 4.2 Show and edit; bundle as one entry or parts

| | A. Bundle one entry only | B. Parts only (flat) | C. One entry with parts, parts also appear in their groups |
|---|---|---|---|
| Matches plugin precedent | Partly | No | Yes (`/plugin` row expandable to components) |
| Skeleton fault | | Duplicates "gortex" | Resolved by linking |
| Per-part toggle | No | Yes | Yes |

Recommended: C. The tool is one entry in the `tools` group with its parts listed and linked; each part is a real row in its own group (MCP server, hooks, skill, env/proxy, daemon) with a "part of gortex" marker and cannot be deleted separately while the bundle owns it. Toggling the bundle toggles its parts; a part can be switched off alone (Graphify's "skill route or MCP route, not both" and Serena's context tool-disable are the reasons). Install state, daemon/proxy status and version come from the box probe. Plugin import (R-SKL-4) splits a plugin into parts under one bundle entry, resolving the silent loss noted in the requirements sweep (a plugin has commands, hooks, sub-agents, MCP, mods; R-SKL-1 holds only skills).

Editing: skills, templates, prompts, commands, personas edit as text with version diff (as skills do today). MCP servers, hooks, env presets edit as forms. Workflows edit in the rataflow graph editor (F:402-413). Bundles are not hand-edited beyond enable/disable and part selection; their definitions are catalogue-supplied (ANA-39). Version history applies to every text kind; run snapshots freeze what a run used.

### 4.3 Attachment to a run

Principle: a run is launched with an exact, frozen set (as `--strict-mcp-config` and `--setting-sources ""` already model), never ambient config.

| Kind | Attach point | Mechanism |
|---|---|---|
| skills | R-SKL-2 levels, unchanged | `{{skills}}` inlining |
| personas | step (workflow) or phase (graph) | persona body plus narrowing of tools/permissions; frozen |
| templates | phase / item kind, library plus project override | `template_name` and version |
| prompts | fixed slots: new-item rewrite, run-start pick | light model; not attachable elsewhere |
| workflows | item pin, else light-model pick, else Settings default; optional target pin | frozen at run start |
| MCP servers | same levels as skills (workspace, project, item-kind, phase) via a generic binding; most specific wins per server name | per-agent renderer; neutral record in, agent-specific config out |
| hooks, commands, sub-agents | same generic binding; delivered only to agents whose adapter declares support | registry row capability flag (R-AGT-5, no per-agent code path) |
| mods | same binding; claude only; delivered only when the agent row opts into extensions | `settings.user_extensions` today is the only lever, so a mod needs a per-run plugin path (unverified feasibility, MOD-96 is rechecking) |
| env presets | target, project, or step; merged in that precedence | `SessionSpec.env` path, secrets by reference |
| bundles | default-on binding (R-LATER-7), project can switch off; parts follow | |

Generic binding: generalise `skill_binding` into one binding shape (kind, ref, level, pinned version or latest, activation `always|glob|off`) so R-SKL-2 semantics are reused rather than reinvented. Reasons are recorded per step as skills do (`trim_record.skill_choices`) so a run explains what it had. R-ID-5 stays intact: everything attached is rendered by htui from Postgres; ambient user extensions stay off unless the registry row opts out.

A capability mismatch (a hook bound to a codex step) is skipped and recorded, never an error that blocks the run, unless the binding is marked required.

### 4.4 Workflows against ANA-33 step graphs

| | A. Workflow replaces step graph | B. Template that graphs instantiate | C. Two layers: workflow is the authored template; ANA-33's run graph is the unrolled, possibly re-planned result |
|---|---|---|---|
| Existing code | Rewrites `step_graph` | Keeps both authored forms | Keeps `step_graph` as the kind default |
| R-ORCH-1 text | Contradicts "beside kinds' graphs" | Matches | Matches |
| Run-time planning (ANA-33) | Needs new home | Planner overrides template | Planner edits the run graph within bounds |

Recommended: C. Facts used: the findings say a run unrolls the template (F:509-512), and R-ORCH-1 keeps kinds' graphs. Inference: both authored forms collapse into one run-graph representation (ANA-33's), so ANA-33 defines the run graph once. A kind's default step graph stays what it is today (linear, per project, edited in Settings per R-TUI-8) and is treated as a degenerate workflow (next links only; gates map to `approved` / `fails`; `retry_limit` to `retries`; `fan_out` to parts). The item's effective workflow resolves: item pin, else light-model pick among workflows, else the kind's default graph. A workflow can therefore be offered to the fast pick or be the kind default; a migration of the existing graphs into the workflow model is optional and left to ANA-33. ANA-33 owns the run-graph storage, the part-to-`run_step` mapping, integrate step and crash recovery. Fan-out in a workflow is divided work, not competition (F:498-501), consistent with R-LATER-13.

Not decided here: whether the planner at run time may add steps beyond the pinned workflow. The setting "run-time changes: ask me or automatic" (F:368) governs workflow switches; plan-level changes inside the run graph are ANA-33's bounds.

### 4.5 Light model and ANA-30

ANA-30 section 7 said no `utility_model` setting and no MOD, because the surviving uses were cheap and surfaces missing. It kept option B (`utility = {agent_id, model}`) in reserve, triggered by a second model-drafting surface. The review adds two surfaces at once: the run-start fast pick and the new-item/requirement rewrite (R-TUI-17). That is the trigger.

What reverses: the "no setting" verdict. A single setting "light model" under Settings > Workflows, default claude haiku, chosen from the models the configured agents offer (F:368). What stays from ANA-30: no model in close-out, prompt assembly, queue order or permission answers (ANA-2 invariant 3, ANA-5 invariant 7); automatic triage still ruled out; MOD-98's wording (no utility_model) needs amending; the "cheap judge" stays rejected. Hence "partly".

Options:

| | A. app_setting key `{agent_id, model}` (ANA-30 B) | B. Registry role (ANA-30 D, `agent.settings.roles`) |
|---|---|---|
| Fit with F | Matches Settings field | Needs UI over registry |
| Cost | `SettingKind` has only Integer/Fraction/Boolean (`prompt/settings.rs:155-189`), so a choice-of-model kind or validator is needed | Data, no new setting kind |
| Fallback | Unset or unavailable: user's own pick | Same |

Recommended: A, as ANA-30 specified, with the new setting kind implemented in the MOD that adds the first surface.

## 5. Verdict details

### 5.1 Storage rule

Postgres holds desired state for every Toolbox kind; the box probe holds observed state; discovery never writes the Toolbox without a person (or the htui agent per R-AGT-11) accepting it. Materialisation to files happens only for the launch of a run, in an htui-owned location or by launch arguments, never into a managed repository (R-ID-4).

### 5.2 Kinds

Eleven kinds, plus bundles as a twelfth entry type: workflows, skills, personas, templates, mods, MCP servers, hooks, commands, sub-agents, prompts, env presets, tools (bundles). "Agent help" is dropped (2.5 item 1). Templates remain a Toolbox group (F:626; MOD-124).

### 5.3 Bundles

One `tools` entry with linked parts (4.2 C). Default-on for coding projects (R-LATER-7); a project may switch a bundle off, so non-coding projects (R-ID-1) are not forced to carry it.

### 5.4 Light model recording and fallback

The pick is stored on the run as the `pick workflow` step (F:324-326): chosen workflow, short reason, model used. On model failure, no configured model, or timeout, the pick falls back to the Settings default workflow, else the kind's default graph; the run never blocks. Nothing but the run's workflow selection reads the pick; the user changes it via `t` in the Backlog before start or by pinning. This satisfies R-ID-6's condition for the pick being item work. The rewrite output is a draft; it is never read until accepted.

### 5.5 Workflow model facts carried from F (unchanged here)

Global templates; typed links; fan-out as a step property; persona and "role" are one thing; personas carry no agent; node labels "persona . agent . model" or "persona . any agent"; a workflow can pin a target; runs keep the version they started with; graceful stop on every step (MOD-125 plan decides ACP and CLI mapping); editor keys and undo depth 100 (F:402-413).

### 5.6 htui agent

Toolbox additions are agent-writable without confirmation (R-AGT-11). Edits of existing entries and deletions ask. This closes the gap that F mentions additions only; recommended in call 5.

## 6. Risks

- Rendering one neutral MCP/hook record to four agent shapes grows a renderer per agent. Mitigation: ship Claude and ACP first (the paths that exist) and mark others unsupported.
- Mods need a per-run plugin load path; today the only lever is a boolean on the agent row, and plugin MCP/agents survived isolation levers (cli/mod.rs ~265, "T1 case 5"). Feasibility unverified.
- Generalising `skill_binding` touches the resolution function used by Pg and mem backends and 199-snapshot UI work (MOD-115); sequence carefully.
- Probe extension beyond `kind:path` may need new probe types; an overlay may only add bare file names today.
- CONCEPTS.md omits the Toolbox and the light model entirely.

## 7. Verdict and phasing

**Verdict: the Toolbox is eleven kinds plus bundles; Postgres is the source of truth for all of them and the box probe only reports observed state; a bundle is one entry with linked, individually toggleable parts; every attachable kind reuses R-SKL-2's levels through one generalised binding and runs receive an exact frozen set; a workflow is the authored template and ANA-33 owns the unrolled run graph, with kinds' step graphs treated as degenerate workflows; and one configurable light model (ANA-30 option B) is built with the first surface that needs it.**

Phasing (proposals; none minted or filed):

1. Phase 1: Toolbox shell with existing kinds (workflows list, skills, personas, templates as groups), light-model setting, workflow editor reading ANA-33's graph model. Needs MOD-115 and ANA-33.
2. Phase 2: generic binding; MCP servers, commands, env presets.
3. Phase 3: bundles with box-probe state (with ANA-39); hooks, sub-agents, mods per agent capability.
4. Project workflows later (ANA-41).

## 8. Proposed MOD items (described only, not filed)

- **P1 - Light model setting and run-start pick.** `app_setting` key `{agent_id, model}`, new setting kind and validator, Settings > Workflows field, fast-pick prompt, recorded `pick workflow` step, deterministic fallback. Amend MOD-98 wording and ANA-30 decision record. Blocked on ANA-33 for the pick's run-graph home.
- **P2 - Generic Toolbox binding.** Generalise `skill_binding` to a typed binding (kind, ref, level, pin, activation), shared Pg/mem resolver, step-level reason record. Requires R-SKL-2 level extension to workspace and item-kind (already requirement, not in code).
- **P3 - MCP server, command, env-preset kinds.** Tables, neutral records, Claude CLI and ACP renderers, secrets by reference, Toolbox forms. Blocked on P2.
- **P4 - Bundle entries and part linking.** Bundle record, part ownership, toggles, probe-reported install/daemon/proxy status; `box_probe_spec` extended beyond `kind:path`. Blocked on ANA-39 and P3.
- **P5 - Hooks, sub-agents, mods as run resources.** Capability flags in the agent registry row, per-run materialisation, skip-and-record on mismatch. Needs a spike on mod loading under isolation.
- **P6 - Plugin import splits into parts.** Extend R-SKL-4 import so a Claude Code plugin yields one bundle with skills, commands, sub-agents, hooks, MCP and mod parts. Blocked on P3-P5.
- **P7 - Kind default graphs as workflows.** Optional migration of `step_graph` into the workflow model; belongs to ANA-33's verdict.
- Requirement edits: define each kind's attachment in R-TUI-7; add the global column to its skill matrix; remove "agent help" from F; amend R-PRM-1's inventory to show where a persona and attached kinds enter the prompt; CONCEPTS.md refresh.

## 9. Open calls for the maintainer

1. Drop "agent help" from the Toolbox? **Recommended: yes**, move it to Settings > Agents.
2. Postgres as desired state, probe as observed state for all kinds? **Recommended: yes.**
3. Bundle as one entry with linked parts, parts toggleable individually? **Recommended: yes.**
4. Generalise the skill binding to all attachable kinds? **Recommended: yes**, with skip-and-record on capability mismatch.
5. Htui agent may add Toolbox entries unasked but must ask before editing or deleting? **Recommended: yes.**
6. Kinds' default step graphs as degenerate workflows, with ANA-33 owning the run graph? **Recommended: yes.**
7. Light model as `app_setting` `{agent_id, model}` (ANA-30 option B), fallback to Settings default workflow then kind default? **Recommended: yes.**
8. R-TUI-7's matrix gets a global column and CONCEPTS.md is refreshed? **Recommended: yes.**
9. Should the pick require acceptance for items in auto mode? **Recommended: no**; it is advisory, recorded and pinnable.

## 10. Not verified

- Full text of R-AGT-5, R-HIS-1, R-SKL-3; the two persona seeds; `claude_emit.rs` contents beyond symbols.
- The rationale behind R-LATER-7's "mandatory" wording and ANA-39's design (not yet read).
- Web claims about Cursor `mcp.json`, Continue Hub and Serena's launch command (recalled); Gortex tool counts as stated by a small-model summary.
- Whether the box probe can be extended to daemons and proxies without a new probe type.
- Whether per-run plugin/mod loading under isolation is achievable on the CLI and ACP paths.

## 11. Sources

- `HANDOFF.md:111-139` (ANA-33, ANA-34), `:184-218` (ANA-40..42), `:698-795` (MOD-115..125), `:516-525` (MOD-98).
- `docs/ui-redesign-findings-2026-10-15.md` sections 4.2a, 4.6, 8, 9, 10.
- `docs/REQUIREMENTS.md` R-TUI-7 (:580-587), R-ORCH-1 (:365-382), R-PRM-4 (:476-478), R-SKL-1/2/4 (:480-488), R-LATER-7 (:688-691), R-ID-6 (:113-121), R-AGT-10 (:344-354), R-AGT-11 (:357-362), R-ORCH-15 (:432-440).
- `docs/ANA-30.md` and `docs/decisions/ana/ana-30.md`; `CONCEPTS.md:35-80`.
- Code: `crates/htui-core/src/model/skill.rs`, `persona.rs`; `crates/htui-store/migrations/0001_init.sql`, `0007_skill_attachments.sql`, `0012_persona.sql`; `crates/htui-agent/src/cli/mod.rs`, `driver.rs`, `box_probe/`, `install/`; `crates/htui/src/ui/tabs/skills/`.
- Branch `spike/ui-skeleton` (`data.rs`, `app.rs`, `views.rs`, `forms.rs`, `modal.rs`).
- Web: code.claude.com/docs/en/plugins, /mcp; github.com/zzet/gortex, chopratejas/headroom, oraios/serena, safishamsi/graphify; Codex, Gemini CLI, Zed, Cursor, Continue docs.

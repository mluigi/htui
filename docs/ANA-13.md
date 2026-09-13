# ANA-13 - Agent and Role Management: oh-my-pi Ideas in htui

> **Scope note:** Deep analysis of how `oh-my-pi` manages different agents and roles, and how its ideas could be implemented in `htui`. Explores dynamic agent spawning, declarative role profiles, and inter-agent communication, contrasting them with `htui`'s current linear-phase model.
>
> **Requirements addressed:** `R-ORCH-*` (Orchestrator capabilities), `R-AGT-*` (Agent models).
>
> **Status (2026-09-13): concluded.** Implementation items spawned as MOD-26 (Declarative Agent Personas) and MOD-27 (Swarm RunKind & `task` MCP Tool).

---

## 1. Context and problem statement

`htui` currently orchestrates agents through strictly linear "step graphs" comprised of ordered phases (e.g., `prd` -> `plan` -> `implement` -> `review`). Agents are essentially wrappers around vendor CLIs (like `claude` or `agy`), and orchestration relies on passing document artifacts sequentially. There is no built-in schema for conversational personas, and multi-agent coordination is strictly limited to isolated fan-outs and manual `Judge` selections.

The `oh-my-pi` repository implements a more dynamic, actor-driven model:
*   **Declarative Roles:** Agents are defined by markdown files featuring frontmatter (model overrides, tool whitelists, output schemas) and a system prompt body.
*   **The `task` Tool:** Agents can dynamically spawn subagents (synchronously or asynchronously), enforce isolated Git worktrees, and validate their structured JSON outputs.
*   **The `hub` Tool:** Agents use an internal IRC-like bus for discovery, messaging, and synchronization, bypassing rigid pipelines.

This document determines how `htui` can adopt these capabilities without violating its existing architectural invariants.

---

## 2. Invariants

1.  **Orchestrator Owns the State Machine:** The orchestrator must still own the step graphs and `RunStep` rows. Any dynamic multi-agent run must map to the existing store structure.
2.  **No `!Send` or Blocking UI:** Subagent execution must map to `tokio` tasks without blocking the main event loop or TUI.
3.  **Strict Capability Enforcement:** If an agent requests to spawn a subagent, the request must be validated against `htui`'s capability checks and security gates (e.g., `RunStep` permissions).
4.  **Artifacts Remain First-Class:** Even in dynamic swarm runs, intermediate artifacts must be captured in the database to support replay and inspection (`R-HIS-1..2`).

---

## 3. Options and Verdicts

### 3.1 Declarative Agent Profiles (Personas)
**Need:** `htui` lacks a way to define custom system prompts, tool whitelists, and model overrides as reusable "Personas" beyond hardcoded seeds.

| Option | Verdict |
|---|---|
| Hardcode personas in `htui-core/seeds` | Rejected. Limits user customizability. |
| Expose a new `TemplateRole::Persona` | Rejected. Overcomplicates the strict phase templates. |
| Adopt Markdown Frontmatter profiles (like `oh-my-pi`) | **Adopted.** Introduce user-level declarative profiles (e.g., `~/.config/htui/agents.d/*.md`). The frontmatter maps directly to `DriverCaps` overrides and `SessionSpec.tools`. |

### 3.2 Dynamic Subagent Spawning (`task` tool)
**Need:** Allow an `implement` agent to dynamically spin up a `reviewer` or `scout` subagent.

| Option | Verdict |
|---|---|
| Modify ACP clients to spawn sub-processes natively | Rejected. Breaks the single-driver invariant and hides compute usage from `htui`'s quota trackers. |
| Add `spawn_subagent` as an MCP tool hosted by `htui` | **Adopted.** Expose `htui`'s orchestrator as an MCP tool (`htui-mcp`). When called, `htui` creates a new `RunStep` linked via a `parent_step_id` and executes it inside the orchestrator task, validating inputs against JSON schemas if provided. |
| Support Isolated Worktrees | **Adopted.** Add `SessionSpec.isolated_worktree` flag. When true, `htui` copies the current repo state, runs the subagent, and surfaces the diff as an `EditProposal` to the parent. |

### 3.3 Multi-Agent Communication (`hub` tool)
**Need:** Break free from strict linear artifact-passing and allow agents to converse in a shared context.

| Option | Verdict |
|---|---|
| IRC Bus via Tool (`oh-my-pi` model) | Rejected. A pure event bus breaks `htui`'s linear step history replay (`R-HIS-2`). |
| Introduce `RunKind::Swarm` | **Adopted.** Alongside `Graph` and `Chat`, create a `Swarm` mode. The orchestrator maintains a single shared context window. Agents yield via a specific tool call, passing the baton back to the orchestrator, which schedules the next active persona. |

---

## 4. Phasing (MOD spawn plan)

This analysis spawns two implementation items to be opened per `lifecycle.md`:

1.  **MOD-26 - Declarative Agent Personas:** Build the Markdown/Frontmatter parser in `htui-core`, add discovery from `~/.config/htui/agents.d/`, and map frontmatter to `SessionSpec` overrides (model, tool whitelist, schema requirements).
2.  **MOD-27 - Swarm RunKind & `task` MCP Tool:** Add `RunKind::Swarm` to `htui-orch`. Implement the `spawn_subagent` MCP tool allowing dynamic `RunStep` creation, JSON schema validation of subagent output, and optional Git worktree isolation.

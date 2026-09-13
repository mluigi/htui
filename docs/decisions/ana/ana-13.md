# ANA-13 - Deep analysis on how oh-my-pi manages different agents and roles (concluded, 2026-09-13)

This item required researching the `oh-my-pi` architecture to extract its agent and role management mechanisms and determining how those patterns could map to `htui`. 

**Findings and Verdict:**
We analyzed both the `htui` model (linear phase orchestration, artifact-driven execution) and the `oh-my-pi` model (declarative subagents spawned via `task` tools, interacting via a `hub` bus).
To incorporate the strengths of `oh-my-pi` without violating `htui`'s invariants (specifically that the orchestrator must maintain strict control of `RunStep` state), we decided to:
1. Adopt Declarative Personas using Markdown frontmatter to configure `SessionSpec` overrides, rather than expanding `htui`'s strict `TemplateRole` system.
2. Introduce a `Swarm` RunKind to `htui-orch` to support dynamic multi-agent execution within a shared step context.
3. Expose `htui`'s orchestration engine to the agents via an MCP tool (`spawn_subagent`) to emulate `oh-my-pi`'s `task` tool, allowing dynamic fan-out with Git worktree isolation and JSON schema validation.

The full design and mapping is recorded in `docs/ANA-13.md`.

**Spawned Items:**
- **MOD-26**: Declarative Agent Personas
- **MOD-27**: Swarm RunKind & `task` MCP Tool

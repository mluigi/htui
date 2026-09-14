# ANA-3 - External context tools (status, 2026-09-14)

## Summary

Concluded the design for integrating external context tools (Headroom, Serena, Graphify, and structural diff) as optional excerpt providers for the prompt builder (`R-LATER-7`). Crucially, implemented the design constraint that agents should never use `grep` or manually get the code, but instead rely on these tools to get exactly the context they need.

Design is documented in `docs/ANA-3.md`.

## What was decided

1. **Provider Impls:** Defined the execution parameters for Headroom, Serena, Graphify, and structural diff, translating their outputs into `ExcerptCandidate`s.
2. **Isolation (`R-ID-4`):** Fixed Serena's state generation by enforcing `--cache-dir <config_dir>/cache/serena/<repo_slug>` outside `repo_box_path`.
3. **Agent Constraint:** Instructs agents via prompt to rely on provided excerpts and prohibits manual `grep`/`find` commands.
4. **No LLM (`R-ID-6`):** Reaffirmed that all providers operate purely deterministically.

## Downstream items

- **MOD-2**: Integrates the providers into `htui-agent`.
- **MOD-11**: The MCP server (`command_run` tool) may enforce command rejection for manual search tools.

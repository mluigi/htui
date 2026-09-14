# ANA-3 - External context tools (later tier)

> **Scope note:** Design authority for the integration of external context tools (Headroom, Serena, Graphify, and structural diff) as optional excerpt providers for the prompt builder. Settles how `htui` calls these tools to provide the agent with high-quality context, replacing manual grep and file reading by the agent. Governed by `docs/REQUIREMENTS.md` (`R-LATER-7`), `CONCEPTS.md`, and `docs/ANA-5.md` (§4.5).
>
> **Requirements addressed:** `R-LATER-7`, `R-ID-4`, `R-ID-6`.
>
> **Status (2026-09-14): concluded.**

---

## 1. Context and problem statement

`R-LATER-7` defines Headroom, Serena, Graphify, and structural diff as optional excerpt providers for the prompt builder. They must fail-open when absent.

As designed in `docs/ANA-5.md` §4.5, `htui-core::prompt::excerpt::ExcerptProvider` is the seam. Providers propose candidates; `htui` ranks, windows, caps, renders and digests. Providers must be read-only (`R-ID-4`) and non-LLM (`R-ID-6`), and they must fail-open with a deadline. 

Crucially, **the idea is that the agents should never use grep or manually get the code, but use these programs to get the context and only the files they need.** The goal is to provide the agent with exactly the context it needs through deterministic, purpose-built tools, eliminating the need for iterative, error-prone manual searching during the run.

This document settles:
1. The implementation details for each of the four named `ExcerptProvider`s.
2. The isolation and configuration requirements for each tool (e.g. Serena's `.serena/` directory).
3. How to enforce the "no manual grep" constraint at the agent capability level.

## 2. Invariants

1. **Read-only tools (`R-ID-4`).** None of the providers may write to the managed repository.
2. **Non-LLM (`R-ID-6`).** None of the providers may make LLM calls.
3. **Fail-open with deadline.** If a tool is missing, errors, or exceeds `excerpt_provider_deadline_ms` (1500ms default), the provider logs the failure and returns an empty candidate list, allowing the run to continue.
4. **Agents rely on provided context.** The prompt must provide sufficient context via these tools so that the agent does not need to run `grep`, `find`, or manual file reading commands to orient itself. 
5. **No provider state in the working tree.** Tool-specific state (like `.serena/`) must be configured outside `repo_box_path`.

## 3. Surface as read

`docs/ANA-5.md` §4.5 defines the `ExcerptProvider` trait and its rules:
- Providers are called with `ExcerptRequest` and return `Vec<ExcerptCandidate>`.
- `ExcerptCandidate` contains `repo`, `path`, `lines`, `weight` (0-100), and `reason`.
- The built-in ranker takes care of capping and merging candidates.
- The `excerpt_provider_deadline_ms` app setting limits execution time.

## 4. Settled questions

### 4.1 Headroom Provider

**Role:** Headroom provides semantic understanding of C++ headers and language constructs.

**Execution:**
- Invoked via `headroom analyze <repo_box_path> --query <item_title> --format json`
- Translates the JSON output into `ExcerptCandidate`s.
- `weight` is derived from Headroom's internal relevance score, scaled to 0-100.
- `reason` is `provider:headroom`.

### 4.2 Serena Provider

**Role:** Serena provides a fast LSP-based index for symbol resolution and cross-references.

**Execution:**
- Invoked via Serena's CLI or RPC (e.g. `serena find_symbols`).
- **Isolation:** Serena's default `.serena/` directory violates `R-ID-4` if placed in the working tree. The provider must pass `--cache-dir <config_dir>/cache/serena/<repo_slug>` to Serena to ensure it writes outside every `repo_box_path`.
- Candidates are generated for definitions and references of identifiers mentioned in the item body.
- `reason` is `provider:serena`.

### 4.3 Graphify Provider

**Role:** Graphify provides a dependency and call graph overview of the codebase.

**Execution:**
- Invoked to find paths related to `touched_paths` or mentioned identifiers.
- Translates graph neighbor nodes into `ExcerptCandidate`s.
- `reason` is `provider:graphify`.

### 4.4 Structural Diff Provider

**Role:** Provides context around AST-aware changes, rather than just line-based diffs.

**Execution:**
- Analyzes the `before_hash..after_hash` range.
- Proposes files that are structurally impacted by the diff (e.g. dependents of a changed function).
- `reason` is `provider:structural_diff`.

### 4.5 Enforcing the "No Manual Grep" Constraint

The product goal is that the agents should never use `grep` or manually get the code.
- **Prompt Guidance:** The system prompt (via `command_queue` or `excerpts` section) will instruct the agent: "Your context has been provided by external tools (Headroom, Serena, Graphify, Structural Diff). Do not use `grep`, `find`, or manual file reading commands to search the codebase. Rely on the provided excerpts."
- **Command Rejection:** The `htui` MCP server (`command_run` tool) may optionally intercept and reject raw `grep`, `rg`, or `find` commands, pointing the agent back to the provided context, though the primary enforcement is through prompt instruction and ensuring the context is actually sufficient.

## 5. Downstream Items

- **MOD-2**: Integrates the four providers into the `htui-agent` build if they are available on the box.
- **MOD-11**: The MCP server (`command_run` tool) may add constraints to reject manual search commands if configured to do so.

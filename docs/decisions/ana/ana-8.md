# ANA-8 - Project and workspace hierarchy model: multi-project workspaces vs global multi-project instance (done, 2026-09-03)

## Summary

Concluded design for `htui`'s top-level entity hierarchy (`workspace` -> `project` -> `repo`), multi-project workspace scoping, cross-project dependency resolution, TUI navigation UX, and schema additions, documented in [`docs/ANA-8.md`](file:///D:/projects/htui/docs/ANA-8.md).

## What was decided

1. **Three-Tier Entity Hierarchy:**
   - **`workspace`:** Operational session and environment container on a development box. Groups 1..N projects, shared toolchains, and active prompt context.
   - **`project`:** Logical software system and planning domain. Owns work item key namespace (`ANA-N`, `MOD-N`), specifications, and business milestones.
   - **`repo`:** Physical Git repository (`.git`, working tree, remote URL). Execution target for commits, code diffs, and local checkout paths (`repo_box_path`).

2. **Workspace-Centric Topology with Fast Switching:**
   - Rejected a heavy global background daemon to prevent RAM bloat, file watcher churn, battery drain, and cross-project prompt/secret contamination.
   - Adopted a lean, workspace-centric execution model with instant (<50ms) workspace switching overlay (`Ctrl-W`).

3. **Workspace-Bounded Lineage & Prompt Injection:**
   - Cross-project dependencies link through UUIDs in `item_link` without schema changes.
   - Prompt injection recursive CTE queries upstream facts and summaries strictly bounded to projects enrolled in the active workspace, emitting concise stubs for external projects.

4. **Multi-Project TUI Layout & Navigation (`MOD-1`):**
   - Handoff tab supports a grouped project tree view and single-project filter toggles.
   - Code Explorer and Diff tabs provide multi-root folder trees and per-repo diff filtering.

5. **PostgreSQL Schema Additions (DDL):**
   - Specified migration `V002__add_workspace_and_project.sql` creating `workspace`, `project`, `workspace_project`, `repo_box_path`, and `workspace_box_path`.
   - Re-anchored `item` to `project_id` with `uq_project_item_key` uniqueness, preserving existing keys without renumbering.

6. **Offline Schemas & Legacy Migration (`MOD-8`):**
   - Retained `<repo>/items.json` (Schema v2) for repository-level offline snapshots.
   - Defined `<workspace>/htui-workspace.json` for multi-repo workspace discovery.
   - Clarified `MOD-8` migration: legacy cross-repo references (`blocked on vfs MOD-2`) map deterministically to `project(slug = 'vfs') -> item(key = 'MOD-2')`.

## Downstream items

Informs the following modules:
- **MOD-1:** TUI scaffold (incorporates workspace/project state and multi-root navigation).
- **MOD-6:** Item store: `items.json` + Postgres (implements V002 DDL and workspace descriptor parsing).
- **MOD-8:** Legacy project migration (uses project slugs for cross-repo pointer resolution).

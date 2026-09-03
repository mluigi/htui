# ANA-8 - Project and workspace hierarchy model: multi-project workspaces vs global multi-project instance

> **Scope note:** Design authority for the top-level entity hierarchy, multi-project and workspace scoping,
> cross-project dependency resolution, navigation UX, and schema additions of `htui`. Governed by
> `.claude/rules/workflow-docs.md` and `CONCEPTS.md`.
>
> **Status (2026-09-03): concluded - design approved. Informs MOD-1 (TUI scaffold), MOD-6 (Item store:
> `items.json` + Postgres), and MOD-8 (Legacy project migration) in `HANDOFF.md`.**

---

## 1. Context and Problem Statement

In [`docs/ANA-1.md`](file:///D:/projects/htui/docs/ANA-1.md), `htui` established its foundational persistence model, box registry,
and zero-agent sync topology. In that initial design, the top-level organizational entity was `repo`:
work items (`item`), atomic facts (`artifact`), and sync states (`sync_state`) were anchored directly to a single Git repository.

While a single-repo anchor is sufficient for isolated utilities, real-world software engineering across
developer workstations and multi-box environments regularly breaks this 1:1 assumption:

1. **Multi-Repo Cohesion:** Complex systems (e.g., game engines, microservices, plugin ecosystems) are partitioned
   across multiple Git repositories (e.g., core engine, virtual file system, asset pipeline, tooling). Developers
   work across these repositories concurrently within a single mental task or feature branch.
2. **Cross-Project Dependencies:** An item in an application repository (e.g., `editor:MOD-4`) frequently depends
   on an item in a shared library repository (e.g., `vfs:MOD-2`). In legacy markdown workflows, developers relied on
   informal text annotations like `blocked on vfs MOD-2`. The persistent system requires rigorous, queryable
   cross-project lineage without creating fragile, tightly coupled monorepo requirements.
3. **Workspace Environment vs. Global Daemon Bloat:** Developers need to group active repositories into a coherent
   workspace sharing toolchains, build profiles, and prompt context. Conversely, running a monolithic global daemon
   that loads, indexes, and watches dozens of unrelated repositories simultaneously causes severe RAM exhaustion,
   file descriptor churn, battery drain, and cross-project secret leakage.
4. **Machine-Specific Path Differences:** A repository or workspace is checked out at `D:\projects\htui` on a Windows
   workstation, `/home/luigi/projects/htui` on a Linux laptop, and `/Users/luigi/htui` on macOS. The data model
   must decouple physical local filesystem locations from logical project identities.
5. **Legacy Markdown Alignment:** Existing codebases employ legacy workflow docs (`HANDOFF.md`, `DECISIONS.md`)
   where item IDs (`ANA-1`, `MOD-6`) are scoped per repository/project. The entity model must cleanly absorb these
   legacy namespaces without collision or loss of identity during migration (`MOD-8`).

This document analyzes and specifies the three-tier entity hierarchy (`workspace` $\to$ `project` $\to$ `repo`),
evaluates the workspace-centric topology against a global daemon, details cross-project dependency resolution and
TUI navigation, provides production-ready PostgreSQL DDL additions, and defines the offline schema formats.

---

## 2. Core Invariants and Architectural Principles

The project and workspace model adheres to five architectural invariants:

1. **The Three-Tier Separation of Concerns:**
   - **`workspace`** is the developer's operational session and environment container. It defines a working set
     of projects, shared context, and active toolchain bindings on a specific machine.
   - **`project`** is the logical software engineering and issue tracking domain. It owns the item key space
     (`ANA-N`, `MOD-N`), specifications, documents, and business milestones.
   - **`repo`** is the physical version control and code storage unit. It owns the Git working tree, remote URL,
     commits, diffs, and local checkout paths.
2. **Workspace-Centric Bounding (No Runaway Global Scope):**
   - Agent prompt assembly, context retrieval, and recursive fact traversal are bounded by the active workspace.
   - Unrelated projects outside the active workspace are never queried, indexed, or injected into prompts, preventing
     token bloat, distraction, and credential cross-contamination.
3. **Physical-Logical Decoupling:**
   - Logical projects and repositories have stable UUIDs and slugs. Physical filesystem paths are bound per box via
     dedicated path mapping tables (`repo_box_path`, `workspace_box_path`), ensuring seamless multi-box sync.
4. **Zero-Overhead Single-Repo Degradation:**
   - A standalone repository with no multi-project requirements operates seamlessly as an implicit single-project
     workspace with zero configuration overhead.
5. **Merge-Friendly Offline Autonomy:**
   - Each physical repository retains its minimal, git-tracked `items.json`. A workspace defines an optional
     root descriptor (`htui-workspace.json`). Both formats allow developers to operate offline on disconnected
     machines without access to Postgres.

---

## 3. Hierarchy & Scope: Workspace, Project, and Repo

### 3.1 Entity Definitions & Responsibilities

```
+-------------------------------------------------------------------------+
|                                WORKSPACE                                |
|  - Developer operational session & active context on a dev box          |
|  - Groups 1..N Projects working together                                |
|  - Defines shared toolchain defaults, prompt boundaries, & TUI state    |
+------------------------------------+------------------------------------+
                                     |
                                     | 1..N (via workspace_project)
                                     v
+-------------------------------------------------------------------------+
|                                 PROJECT                                 |
|  - Logical product / component / planning boundary                      |
|  - Owns item key namespace (ANA-N, MOD-N) & milestones                  |
|  - Owns macro documents (CONCEPTS.md) & architectural invariants        |
+------------------------------------+------------------------------------+
                                     |
                                     | 1..N (Primary + Subordinates)
                                     v
+-------------------------------------------------------------------------+
|                                  REPO                                   |
|  - Physical Git repository (.git, working tree, branches, commits)      |
|  - Target of code edits, diffs, test execution, & git hooks             |
|  - Hosts derived ./items.json for offline snapshot                      |
+-------------------------------------------------------------------------+
```

| Entity | Canonical Identity | Primary Responsibilities | Physical Representation |
|---|---|---|---|
| **`workspace`** | `UUID`, `slug` (e.g. `dingine-dev`, `htui-core`) | Session grouping, multi-project navigation, active prompt boundary, local box root path | `htui-workspace.json` (optional) or Postgres |
| **`project`** | `UUID`, `slug` (e.g. `htui`, `dingine`, `vfs`) | Work item ownership, unique item key space (`ANA-1`), milestone tracking, issue tracker mapping | Logical entity in DB; mapped to primary repo |
| **`repo`** | `UUID`, `name` (e.g. `htui`, `vfs`) | Git operations, diff inspection, working tree files, commit history, git hooks | Git working tree root on disk + `<repo>/items.json` |

### 3.2 Cardinality and Mapping Scenarios

The relationship between `workspace`, `project`, and `repo` supports three distinct real-world patterns:

#### Scenario A: Standard 1:1:1 Single-Repo Project (e.g. `htui`)
* A developer works on an independent application.
* **Workspace:** `htui` (implicit or explicit).
* **Project:** `htui` (owns items `ANA-1`, `MOD-1`, etc.).
* **Repo:** `htui` (points to `git.mluigi.it/htui.git`, local path `D:\projects\htui`).
* **Behavior:** Zero friction. The user opens the repo; `htui` auto-resolves the workspace and project as 1:1.

#### Scenario B: Multi-Repo System / Modular Workspace (e.g. `dingine`)
* A developer works on an engine system comprising a core engine repo, a virtual file system library repo, and an asset pipeline repo.
* **Workspace:** `dingine-engine` (contains projects `dingine`, `vfs`, `asset-pipeline`).
* **Projects:**
  - Project `dingine` $\to$ Primary Repo `dingine`
  - Project `vfs` $\to$ Primary Repo `vfs`
  - Project `asset-pipeline` $\to$ Primary Repo `asset-pipeline`
* **Behavior:** The developer opens `htui` in the `dingine-engine` workspace. The TUI displays items across all three
  projects in the Handoff Tab. Edits to `vfs` execute against the `vfs` git repo, while dependencies between
  `dingine:MOD-12` and `vfs:MOD-3` resolve transparently.

#### Scenario C: Shared Common Library in Multiple Workspaces
* The shared `vfs` project is used by both the `dingine-engine` workspace and an independent `tools-suite` workspace.
* `workspace_project` maps `vfs` into both workspaces.
* Work items for `vfs` remain unified in Postgres and sync to `vfs/items.json`, regardless of which workspace
  session the developer is actively running.

---

## 4. Topology Evaluation: Workspace-Centric Model vs. Global Instance

A central architectural question is whether `htui` should operate as a **single global background daemon**
managing all projects simultaneously across boxes, or as a **workspace-centric client with fast switching**.

### 4.1 Comparative Evaluation

| Dimension | Global Multi-Project Daemon | Workspace-Centric Model (Recommended) |
|---|---|---|
| **System Resource Footprint** | **Poor:** Must maintain active file watchers (`ReadDirectoryChangesW`/`inotify`), AST caches, and git status for dozens of repos simultaneously. High idle RAM (1–2 GB) and battery drain. | **Optimal:** Watches only the active workspace's repos (typically 1–4 repos). Idle RAM <50 MB; minimal battery impact on laptops. |
| **Architectural Alignment** | **Violates Invariants:** `CONCEPTS.md` explicitly mandates: *"It is not an IDE, not a general terminal multiplexer, and not a cloud-hosted orchestration daemon. It is a local-first, developer-guided agent harness."* | **Strict Compliance:** Keeps `htui` a fast, focused, local-first interactive terminal binary. |
| **Token Economy & Prompt Focus** | **High Risk:** Global search/grepping risks pulling irrelevant files, artifacts, or secrets from unrelated client projects into agent prompts. | **Strictly Bounded:** Prompts and recursive CTE traversals are quarantined to projects within the active workspace. |
| **Offline & Network Resilience** | **Fragile:** If Postgres or network shares are unavailable, a global daemon stalls or throws cascades of background reconnect errors. | **Robust:** Opens the local workspace immediately in offline read-only mode using local `items.json` and cached files. |
| **Secret Isolation** | **Leaky:** Environment secrets, API keys, and repository paths for all projects coexist in a single process memory space. | **Isolated:** Only credentials relevant to the active workspace projects are unlocked or scrubbed. |
| **Context Switching Speed** | Sub-millisecond (everything already in RAM), but at massive continuous resource cost. | **<50 milliseconds:** Fast workspace switching in TUI via indexed SQLite/Postgres queries and cached git handles. |

### 4.2 Verdict: Workspace-Centric Model with Fast Switching

`htui` strictly adopts the **Workspace-Centric Model**.
- The TUI process runs in the context of an active workspace.
- The active workspace is inferred from the current working directory, specified via `--workspace <name|path>`,
  or selected from a fast interactive switcher in the TUI (`Ctrl-W`).
- Switching workspaces completely swaps the active project set, re-binds git working trees, and clears active
  agent execution memory, providing clean isolation and maximum responsiveness.

---

## 5. Cross-Project Dependencies and Recursive Lineage

### 5.1 Qualified Item Keys & Scoping

In single-repo workflows, items are referenced simply by their key: `ANA-1`, `MOD-6`.
In a multi-project workspace, references require unambiguous qualification:

1. **Unqualified Key (`MOD-4`):** Implicitly resolves to the *current active project* in the TUI context.
2. **Project-Qualified Key (`vfs:MOD-2` or `dingine:ANA-1`):** Resolves to item `MOD-2` owned by project with slug `vfs`.
3. **Fully Qualified Global Key (`workstation/vfs:MOD-2`):** Used only in cross-box diagnostics; within `htui`,
   project slugs are globally unique within Postgres.

### 5.2 Directed Edge Resolution in `item_link`

In [`docs/ANA-1.md`](file:///D:/projects/htui/docs/ANA-1.md), `item_link` was defined with UUID foreign keys:
```sql
CREATE TABLE item_link (
    from_item_id    UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    to_item_id      UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL, -- 'blocked_by', 'origin', 'relates', 'supersedes'
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (from_item_id, to_item_id, kind)
);
```

Because `item_link` connects items via UUIDs, **the edge table inherently supports cross-project links without schema alteration**.
An item in Project A (`htui`) linking `blocked_by` an item in Project B (`vfs`) is simply a directed edge between their two item UUIDs.

### 5.3 Workspace-Bounded Prompt Injection Query (Recursive CTE)

When `htui` constructs an agent's initial prompt, it traverses prerequisite items to gather distilled summaries
and high-importance facts (`importance >= 7`). In a multi-project ecosystem, traversing unconstrained edges could
pull irrelevant facts from distant projects.

`htui` applies a **workspace boundary filter** to the recursive CTE:
- The CTE traverses `blocked_by` and `origin` edges up to 2 hops.
- High-importance facts (`artifact`) and distilled documents (`item_document`) are gathered **only from projects
  enrolled in the active workspace**.
- Prerequisite items outside the active workspace are rendered as compact external status lines (e.g.
  `[External Prerequisite: auth-service:MOD-1 (status: done)]`) without inlining full artifacts.

```sql
-- Workspace-Bounded Prerequisite Fact & Document Injection CTE
WITH RECURSIVE upstream_graph AS (
    -- Anchor: direct prerequisites
    SELECT 
        il.from_item_id,
        il.to_item_id,
        il.kind,
        1 AS depth
    FROM item_link il
    WHERE il.from_item_id = :target_item_id
      AND il.kind IN ('blocked_by', 'origin')
    
    UNION ALL
    
    -- Recursive step: 2nd hop prerequisites
    SELECT 
        il.from_item_id,
        il.to_item_id,
        il.kind,
        ug.depth + 1
    FROM item_link il
    JOIN upstream_graph ug ON il.from_item_id = ug.to_item_id
    WHERE ug.depth < 2
      AND il.kind IN ('blocked_by', 'origin')
)
-- 1. High-importance atomic facts from upstream items within the active workspace
SELECT 
    'fact' AS entry_type,
    p.slug || ':' || i.item_key AS qualified_key,
    a.headline,
    a.content,
    a.importance,
    a.commit_hash
FROM upstream_graph ug
JOIN item i ON i.id = ug.to_item_id
JOIN project p ON p.id = i.project_id
JOIN workspace_project wp ON wp.project_id = p.id AND wp.workspace_id = :active_workspace_id
JOIN artifact a ON a.item_id = i.id
WHERE a.importance >= 7

UNION ALL

-- 2. Distilled summaries from upstream items within the active workspace
SELECT 
    'summary' AS entry_type,
    p.slug || ':' || i.item_key AS qualified_key,
    d.title AS headline,
    d.content,
    10 AS importance,
    NULL AS commit_hash
FROM upstream_graph ug
JOIN item i ON i.id = ug.to_item_id
JOIN project p ON p.id = i.project_id
JOIN workspace_project wp ON wp.project_id = p.id AND wp.workspace_id = :active_workspace_id
JOIN item_document d ON d.item_id = i.id AND d.kind = 'summary'
WHERE d.version = (
    SELECT MAX(version) FROM item_document WHERE item_id = i.id AND kind = 'summary'
)

UNION ALL

-- 3. Stubs for prerequisites outside the active workspace (no deep fact injection)
SELECT 
    'external_stub' AS entry_type,
    p.slug || ':' || i.item_key AS qualified_key,
    i.title AS headline,
    'Status: ' || i.status || ' (External project outside active workspace)' AS content,
    5 AS importance,
    NULL AS commit_hash
FROM upstream_graph ug
JOIN item i ON i.id = ug.to_item_id
JOIN project p ON p.id = i.project_id
WHERE NOT EXISTS (
    SELECT 1 FROM workspace_project wp 
    WHERE wp.project_id = p.id AND wp.workspace_id = :active_workspace_id
)
ORDER BY importance DESC;
```

---

## 6. TUI Navigation & Multi-Project UX (`MOD-1`)

The introduction of workspaces and projects updates the TUI interface (`ratatui`) layout and keybinds
without adding clutter.

### 6.1 Top Bar & Status Header

The top bar displays the operational coordinates:
```
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│ htui v0.1.0 │ Workspace: dingine-engine [Ctrl-W] │ Box: DESKTOP-RIG │ Sync: Clean (Postgres TLS) │
├──────────────┬──────────────────┬────────────────┬───────────────┬───────────────────────────────┤
│ 1: Handoff   │ 2: Code Explorer │ 3: Active Diff │ 4: Agent Chat │ 5: Settings                   │
└──────────────┴──────────────────┴────────────────┴───────────────┴───────────────────────────────┘
```

### 6.2 Handoff Tab Layout in Multi-Project Mode

When a workspace contains multiple projects, the Handoff Tab left pane offers two toggleable view modes
(toggled via `P` or `Ctrl-F`):

#### View Mode 1: Grouped Tree View (Default)
```
┌─ Work Items ──────────────────────────────────────────┬─ [dingine:MOD-4] Orchestrator Pipeline ─────┐
│ ▼ dingine (Core Engine)                    [3 open]   │ Status: open          Version: 3            │
│   • ANA-2 - Orchestration pipeline design             │ Target Repo: dingine (branch: main)         │
│   • MOD-1 - TUI scaffold                              │ Target Box:  DESKTOP-RIG (Windows x86_64)   │
│   • MOD-4 - Orchestrator step engine                  ├─────────────────────────────────────────────┤
│ ▼ vfs (Virtual File System)                [1 open]   │ Prerequisite Lineage:                       │
│   • MOD-2 - Memory-mapped file reader                 │   ✔ ANA-2 Orchestration design (done)       │
│ ▶ asset-pipeline                           [0 open]   │   ⏳ vfs:MOD-2 Mmap reader (in_progress)     │
│                                                       ├─────────────────────────────────────────────┤
│                                                       │ Specification:                              │
│                                                       │ Implement the step graph state machine...   │
│                                                       │                                             │
│ [Enter] Run Item   [Space] Detail   [N] New Item      │ [Actions: (R)un | (E)dit | (D)iff | (P)hase]│
└───────────────────────────────────────────────────────┴─────────────────────────────────────────────┘
```

#### View Mode 2: Filtered Flat List
The user presses `1`, `2`, `3` to filter items strictly to a single project, or `0` for all projects combined,
sorted by urgency and dependency readiness.

### 6.3 Fast Workspace Switcher Overlay (`Ctrl-W`)

Pressing `Ctrl-W` from any tab opens an instant modal overlay:
```
┌─ Switch Workspace ────────────────────────────────────────────────────────┐
│  > 1. dingine-engine    (D:\projects\dingine)              [4 open items] │
│    2. htui-core         (D:\projects\htui)                 [15 open items]│
│    3. logging-lib       (D:\projects\libs\logging)         [1 open item]  │
│    4. [Open from Path...]                                                 │
│                                                                           │
│  [Up/Down] Select   [Enter] Switch Workspace   [Esc] Cancel               │
└───────────────────────────────────────────────────────────────────────────┘
```
Switching workspaces takes <50ms. The TUI reloads the active project set, refreshes `items.json` handles,
re-binds the working tree diff watchers, and returns the user to the Handoff tab.

### 6.4 Code Explorer & Diff Tab Multi-Root Behavior (`MOD-3`)

- **Code Explorer (`Tab 2`):** In a multi-repo workspace, the root of the file explorer displays a multi-root
  tree identical to VS Code or Cargo workspaces:
  ```
  ▼ dingine/
    ▼ src/
      main.rs
  ▼ vfs/
    ▼ src/
      lib.rs
  ```
- **Diff Tab (`Tab 3`):** Displays a project filter bar at the top (`[All Repos | dingine (+12/-3) | vfs (+45/-0)]`).
  The user can review diffs across all modified repositories in the workspace before approving steps.

---

## 7. PostgreSQL Schema Specification (DDL)

To extend the core schema defined in `ANA-1`, the following migration DDL establishes the `workspace` and `project`
entities, machine path bindings, and updates the `item` and `artifact` tables.

```sql
-- htui PostgreSQL Schema DDL
-- Migration: V002__add_workspace_and_project.sql

-- ============================================================================
-- 1. Workspaces (Developer Session & Grouping Layer)
-- ============================================================================
CREATE TABLE workspace (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL UNIQUE,          -- Human display name, e.g. 'Dingine Engine Workspace'
    slug            TEXT NOT NULL UNIQUE,          -- CLI/URL identifier, e.g. 'dingine-engine', 'htui'
    description     TEXT NOT NULL DEFAULT '',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_workspace_slug ON workspace(slug);

-- ============================================================================
-- 2. Projects (Logical Planning & Key Domain)
-- ============================================================================
CREATE TABLE project (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL UNIQUE,          -- e.g. 'Dingine Core', 'Virtual File System', 'htui'
    slug            TEXT NOT NULL UNIQUE,          -- Prefix identifier, e.g. 'dingine', 'vfs', 'htui'
    description     TEXT NOT NULL DEFAULT '',
    primary_repo_id UUID REFERENCES repo(id) ON DELETE SET NULL, -- Default target repo for code
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_project_slug ON project(slug);

-- ============================================================================
-- 3. Workspace-to-Project Membership (Many-to-Many)
-- ============================================================================
CREATE TABLE workspace_project (
    workspace_id    UUID NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    project_id      UUID NOT NULL REFERENCES project(id) ON DELETE CASCADE,
    display_order   INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, project_id)
);

CREATE INDEX idx_workspace_project_proj ON workspace_project(project_id);

-- ============================================================================
-- 4. Machine-Specific Path Bindings (Multi-Box Support)
-- ============================================================================
-- Maps a physical repository's root checkout directory on a specific dev machine
CREATE TABLE repo_box_path (
    repo_id         UUID NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    box_id          UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    local_path      TEXT NOT NULL,                 -- e.g. 'D:\projects\htui' or '/home/luigi/projects/htui'
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (repo_id, box_id)
);

-- Maps a workspace root directory on a specific dev machine (where htui-workspace.json resides)
CREATE TABLE workspace_box_path (
    workspace_id    UUID NOT NULL REFERENCES workspace(id) ON DELETE CASCADE,
    box_id          UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    root_path       TEXT NOT NULL,                 -- e.g. 'D:\projects\dingine-workspace'
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, box_id)
);

-- ============================================================================
-- 5. Schema Adjustments to Core Tables from ANA-1
-- ============================================================================

-- (A) Alter item table: re-anchor to project_id, retain repo_id as execution target
ALTER TABLE item ADD COLUMN project_id UUID REFERENCES project(id) ON DELETE CASCADE;

-- Backfill project_id for existing installations: create 1:1 project per repo
INSERT INTO project (id, name, slug, primary_repo_id)
SELECT id, name, LOWER(name), id FROM repo
ON CONFLICT DO NOTHING;

UPDATE item SET project_id = repo_id WHERE project_id IS NULL;
ALTER TABLE item ALTER COLUMN project_id SET NOT NULL;

-- Update uniqueness constraint: item_key is unique PER PROJECT (not per repo)
ALTER TABLE item DROP CONSTRAINT IF EXISTS uq_repo_item_key;
ALTER TABLE item ADD CONSTRAINT uq_project_item_key UNIQUE (project_id, item_key);

-- Make repo_id nullable on item (defaults to project.primary_repo_id)
ALTER TABLE item ALTER COLUMN repo_id DROP NOT NULL;

CREATE INDEX idx_item_project_status ON item(project_id, status);

-- (B) Alter artifact table: allow project-level facts as well as repo-level facts
ALTER TABLE artifact ADD COLUMN project_id UUID REFERENCES project(id) ON DELETE CASCADE;
UPDATE artifact SET project_id = (SELECT project_id FROM item WHERE item.id = artifact.item_id)
WHERE item_id IS NOT NULL;
CREATE INDEX idx_artifact_project_importance ON artifact(project_id, importance DESC);
```

---

## 8. Offline Representation: `items.json` and `htui-workspace.json`

To preserve `htui`'s offline-first and merge-friendly invariants across all boxes, local representations
are clean, minimal, and git-trackable.

### 8.1 Repository Snapshot: `<repo_root>/items.json` (Schema v2)

The repository-level `items.json` file remains located at the root of each Git repository.
Schema v2 adds explicit `project` qualification while remaining backwards-compatible with v1 readers:

```json
{
  "$schema": "https://json.schemastore.org/htui-items-v2.json",
  "version": 2,
  "project": "htui",
  "repo": "htui",
  "items": [
    {
      "uuid": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
      "key": "ANA-1",
      "title": "Data model, box registry and sync topology",
      "status": "done"
    },
    {
      "uuid": "4f2d3e1a-8c5b-4299-bb11-9a7e3d8f2b4c",
      "key": "MOD-1",
      "title": "TUI scaffold",
      "status": "open"
    },
    {
      "uuid": "d8e3a2b1-5f4c-4e89-a123-bc9876543210",
      "key": "ANA-8",
      "title": "Project and workspace hierarchy model: multi-project workspaces vs global multi-project instance",
      "status": "done"
    }
  ]
}
```

### 8.2 Workspace Descriptor: `<workspace_root>/htui-workspace.json` (Optional)

In multi-repo setups, an optional `htui-workspace.json` file at the root folder defines the workspace
and relative repository paths. This file is modeled after `.code-workspace` and Cargo `[workspace]`:

```json
{
  "$schema": "https://json.schemastore.org/htui-workspace-v1.json",
  "version": 1,
  "name": "Dingine Engine Workspace",
  "slug": "dingine-engine",
  "projects": [
    {
      "name": "Dingine Core",
      "slug": "dingine",
      "path": "./dingine"
    },
    {
      "name": "Virtual File System",
      "slug": "vfs",
      "path": "./libs/vfs"
    },
    {
      "name": "Asset Pipeline",
      "slug": "asset-pipeline",
      "path": "./tools/asset-pipeline"
    }
  ],
  "settings": {
    "default_toolchain": "clang-18",
    "prompt_budget_tokens": 12000
  }
}
```

### 8.3 Workspace Discovery Logic on Startup

When `htui` launches in directory `D`, it discovers its operational context using a 4-stage resolution ladder:

1. **CLI Flag Override:** If `--workspace <name|path>` or `--project <slug>` is passed, bind explicitly.
2. **Workspace Root File:** Check if `D/htui-workspace.json` (or any parent directory up to Git root / filesystem root)
   exists. If found, load the multi-project workspace definition.
3. **Repository Root Snapshot:** Check if `D/items.json` or `D/.git` exists. If found, treat the directory as an
   isolated single-project workspace (Workspace name = Project name = Repo name).
4. **Box Path Registry in Postgres:** Query `workspace_box_path` and `repo_box_path` matching `D` on the current box
   hostname (`box.hostname`).
5. **Fallback:** If completely uninitialized, offer an interactive wizard: `(1) Initialize single-repo project` or
   `(2) Create multi-project workspace`.

---

## 9. Migration Strategy & Impact on Legacy Importer (`MOD-8`)

In [`HANDOFF.md`](file:///D:/projects/htui/HANDOFF.md), item `MOD-8` specifies the legacy markdown workflow doc importer (`HANDOFF.md`,
`DECISIONS.md`, `docs/decisions/**`, `docs/ANA-*.md`).

The workspace and project model directly clarifies and simplifies `MOD-8`:

### 9.1 Resolving Legacy Cross-Repo Annotations

In legacy markdown repositories (governed by `.claude/rules/workflow-docs.md`), cross-repo references were
written as:
- `blocked on vfs MOD-2`
- `originates from engine ANA-11`

Under `ANA-1` (single repo), cross-repo links required fuzzy repository matching. Under `ANA-8`:
1. `MOD-8` creates a `project` for each imported repository (`name = repo_name`, `slug = repo_slug`).
2. When parsing `blocked on <target_repo> <KEY>`, `MOD-8` resolves `<target_repo>` directly to `project(slug = target_repo)`
   and queries `item(project_id, item_key = KEY)`.
3. An `item_link` edge (`kind = 'blocked_by'`) is inserted between the two UUIDs.
4. If the target project has not been imported yet, `MOD-8` creates an unverified stub item row
   (`status = 'open'`, `title = 'Import Stub'`), which automatically links and updates when the target repository
   is subsequently imported.

### 9.2 Zero ID Renumbering

Because `item_key` is unique per `project_id` (`CONSTRAINT uq_project_item_key UNIQUE (project_id, item_key)`),
**every existing item ID (`ANA-1`, `MOD-1`, `CLEAN-3`) is preserved verbatim**. No keys are renumbered,
no git commit messages referencing old keys are invalidated, and all legacy documentation links remain valid.

---

## 10. Summary and Implementation Phasing

### 10.1 Key Decisions Summary

1. **Adopt Three-Tier Entity Hierarchy:** Formalized `workspace` (session/box context) $\to$ `project` (planning/key domain)
   $\to$ `repo` (git storage/execution target).
2. **Select Workspace-Centric Topology:** Rejected global background daemon in favor of a lean, fast-switching
   workspace model, protecting laptop battery, RAM, and prompt token boundaries.
3. **Workspace-Bounded Prompt CTE:** Directed edges in `item_link` traverse cross-project dependencies via UUIDs,
   with prompt injection strictly bounded to projects enrolled in the active workspace.
4. **Multi-Project TUI Layout:** Enhanced `ratatui` scaffold (`MOD-1`) with a grouped Handoff tree, multi-root
   code explorer, and `<50ms` workspace switching overlay (`Ctrl-W`).
5. **Backwards-Compatible Offline Storage:** Retained `<repo>/items.json` for merge-friendly repository snapshots
   and specified `<workspace>/htui-workspace.json` for multi-repo workspace coordination.

### 10.2 Downstream Module Impact

- **`MOD-1` (TUI scaffold):** Incorporates `workspace` and `project` selector state into the `ratatui` state model;
  supports grouped tree rendering in Handoff Tab and multi-root tree in Code Explorer.
- **`MOD-6` (Item store: `items.json` + Postgres):** Implements migration `V002__add_workspace_and_project.sql`,
  enforces `uq_project_item_key`, and handles `htui-workspace.json` parsing.
- **`MOD-8` (Legacy project migration):** Uses project slugs to deterministically resolve legacy cross-repo
  pointers (`blocked on vfs MOD-2`) into typed `item_link` edges without renumbering keys.

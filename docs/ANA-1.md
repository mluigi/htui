# ANA-1 - Data model, box registry, and sync topology

> **Superseded (2026-09-03):** `docs/REQUIREMENTS.md` replaces this document where they conflict.
> Superseded here: the DDL, `items.json`, `artifact`/`artifact_link`, timestamp LWW conflict rule,
> `sync_state`, `external_issue_mapping`. Surviving, as restated in REQUIREMENTS §2, §8, §9: box
> registry and prompt injection, transcript split, local scrubbing. Schema v2 is ANA-9. Kept as
> history; do not implement from this file.
>
> **Scope note:** Design authority for the persistence layer, machine profiling, and synchronization
> architecture of `htui`. Governed by `.claude/rules/workflow-docs.md` and `CONCEPTS.md`.
>
> **Status (2026-09-03): concluded - design approved. Implementation tracked as MOD-6 (Item store:
> `items.json` + Postgres), MOD-7 (Box registry + prompt injection), and MOD-5 (OneDev issue sync)
> in `HANDOFF.md`.**

---

## 1. Context and Problem Statement

`htui` is a local-first Terminal User Interface written in Rust (`ratatui`, `crossterm`, `tokio`)
that wraps AI coding agents (`claude`, `agy`) to manage handoff workflows, step graphs, and execution
across multiple repositories and development machines.

Before implementing storage or agent execution code, `htui` requires a rigorous persistent data model
and synchronization topology. Today, developer-agent workflows suffer from five foundational problems:

1. **State Isolation across Dev Boxes:** Developers switch between machines (desktop workstation,
   laptop, remote devbox). When task state and history live only on local disk or inside session
   memory, handoff context is lost, duplicated, or stale.
2. **Machine Toolchain Divergence:** Agents repeatedly trip over box-specific build quirks (e.g.
   MSVC `cl.exe` vs MinGW GCC 14.2 on Windows; differing CMake versions; presence or absence of
   vcpkg; shell fork failures). Without structured knowledge of the host box, agents generate
   invalid build commands and waste turns debugging machine configuration.
3. **Context Inflation from Raw Transcripts:** Naive agent harnesses inject entire previous conversation
   transcripts into subsequent prompts. A single raw session transcript easily reaches 50–100 KB,
   burning context windows, increasing latency, and diluting reasoning focus.
4. **Git Merge Friction on Shared State:** Storing full task descriptions, human discussions, and
   execution metadata in git-tracked repository files creates frequent merge conflicts across feature
   branches and concurrent box runs.
5. **Offline/Online Disconnect:** A purely cloud-hosted database blocks development on planes or
   isolated networks, whereas a purely local file store prevents multi-box coordination.

This document establishes the persistent layer architecture, schema DDL, machine registry, transcript
privacy model, and synchronization rules to resolve these challenges.

---

## 2. Core Invariants and Architectural Principles

The persistent layer adheres to six non-negotiable invariants defined in `CONCEPTS.md`:

1. **Single-Canonical Ownership by Domain:**
   * **Postgres** is canonical for core work item state, revision history, dependency graphs, box profiles,
     execution runs, phase documents (`item_document`), and atomic repository facts (`artifact`).
   * **`items.json`** in managed repos is a derived, merge-friendly offline snapshot containing only
     minimal identity and status fields (`uuid`, `key`, `title`, `status`). It never carries task bodies,
     discussions, or facts.
   * **External Issue Tracker (OneDev / Git issues)** is canonical for human-facing story, comments,
     and discussion threads (pluggable and decoupled pending `ANA-6`).
2. **The Zero-Agent Sync Invariant:**
   * Synchronization between Postgres, local repository snapshots, and external issue trackers is
     executed by pure, deterministic API clients.
   * **No LLM agent is ever in the sync path**, preserving token consumption strictly for coding tasks.
3. **Macro Documents vs. Atomic Repository Facts:**
   * Full deliverables (PRD, Plan, Review, Summary) live in a versioned `item_document` table.
   * Granular codebase truths, compiler quirks, and discoveries live in an `artifact` table as atomic
     facts with an explicit `importance` score (1–10), pinned to a git `commit_hash`.
   * Prompts inject only the active phase document plus prioritized facts (`importance >= 7`), keeping
     context windows lean, dense, and verifiable.
4. **Knowledge Graph Lineage & Traversal:**
   * Work item dependencies are directed edges in `item_link` using explicit `blocked_by` semantics
     (`from_item_id` is blocked by `to_item_id`).
   * Repository facts connect to each other in an `artifact_link` graph (`supports`, `refines`,
     `contradicts`, `relates_to`).
   * Traversal is performed via recursive SQL Common Table Expressions (CTEs), eliminating graph DB overhead.
5. **Application-Managed Revision History:**
   * All mutations to item specifications (`title`, `body`) are recorded in an `item_revision` table
     managed by the application layer (`htui`), capturing author machine ID and semantic reason to power
     3-way diff conflict resolution.
6. **Local-First Secret Scrubbing:**
   * All raw transcripts and tool outputs must be scrubbed of credentials, API keys, and environment
     secrets locally on the host machine before any data leaves the box.

---

## 3. Storage Hierarchy and Sync Topology

`htui` operates as a **hub-and-spoke with local offline caching**:

```
                  +-----------------------------------+
                  |        External Issue Tracker     |
                  |   (OneDev / GitHub / Linear API)  |
                  |   Canonical for Human Discussion  |
                  +-----------------+-----------------+
                                    ^
                   Pure REST Client | (Zero LLM Tokens)
                                    v
+-----------------------------------+-----------------------------------+
|                     Central Postgres Database                         |
|  Canonical for: Items, Revisions, Box Registry, Runs, Steps,          |
|  Phase Documents, Atomic Facts, Links, and Sync State                 |
+-------------------+-------------------------------+-------------------+
                    ^                               ^
   SQL / TLS Sync   |                               | SQL / TLS Sync
   (LWW + Divergence|                               | (LWW + Divergence)
                    v                               v
+-------------------+-----------+   +---------------+-------------------+
|      Box 1 (Workstation)      |   |        Box 2 (Laptop)             |
|                               |   |                                   |
| - Local htui TUI Engine       |   | - Local htui TUI Engine           |
| - Box Profiler (hardware/env) |   | - Box Profiler (hardware/env)     |
| - Offline Cache & Write Queue |   | - Offline Cache & Write Queue     |
| - Local Scrubbing Pipeline    |   | - Local Scrubbing Pipeline        |
| - Managed Repo:               |   | - Managed Repo:                   |
|   `./items.json` (Snapshot)   |   |   `./items.json` (Snapshot)       |
+-------------------------------+   +-----------------------------------+
```

### 3.1 Domain Ownership Matrix

| Field / Entity | Canonical Source | Secondary Replica | Sync Direction & Trigger |
|---|---|---|---|
| Item Key (`ANA-1`, `MOD-6`) | Postgres (`item`) | `items.json` | Postgres → `items.json` on item creation |
| Item Title & Status | Postgres (`item`) | `items.json`, Issue Tracker | Postgres ↔ `items.json` (LWW) → Issue Tracker |
| Item Body / Technical Specs | Postgres (`item`) | Local RAM / DB Cache | Postgres only (never in `items.json`) |
| Item Revision History | Postgres (`item_revision`) | None | App writes revision on every mutation |
| Item Dependency Edges | Postgres (`item_link`) | None | Postgres only (`blocked_by`, `origin`, etc.) |
| Box Profiles & Toolchains | Postgres (`box`) | Local `~/.config/htui/box.json` | Box probe → Postgres on session start |
| Execution Pipelines & Steps | Postgres (`run`, `run_step`) | Local audit logs | Local execution → Postgres |
| Macro Phase Documents | Postgres (`item_document`) | Local memory / prompt cache | Agent output → Postgres |
| Atomic Repository Facts | Postgres (`artifact`) | None (prompt-injected) | Agent discovery → Postgres |
| Fact Knowledge Graph | Postgres (`artifact_link`) | None | Postgres only |
| Human Discussion / Comments | Issue Tracker | Postgres (cached) | Issue Tracker ↔ Local UI (pending `ANA-6`) |
| Raw Debug Transcripts | Box disk (`file://`) | Issue / S3 Attachment URI | Scrubbed upload on step completion |

---

## 4. PostgreSQL Schema Specification (DDL)

The schema is normalized, indexed for rapid TUI rendering, and enforces referential integrity.

```sql
-- htui PostgreSQL Core Schema DDL
-- Migration: V001__init_core_schema.sql

CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ============================================================================
-- 1. Repositories
-- ============================================================================
CREATE TABLE repo (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name            TEXT NOT NULL UNIQUE,          -- e.g. 'htui', 'ding', 'logging'
    remote_url      TEXT,                          -- e.g. 'https://git.mluigi.it/htui.git'
    default_branch  TEXT NOT NULL DEFAULT 'main',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_repo_name ON repo(name);

-- ============================================================================
-- 2. Box Registry
-- ============================================================================
CREATE TABLE box (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    hostname        TEXT NOT NULL UNIQUE,          -- e.g. 'DESKTOP-RIG', 'X1-CARBON'
    os_family       TEXT NOT NULL,                 -- 'windows', 'linux', 'macos'
    os_version      TEXT NOT NULL,                 -- Kernel/build version
    arch            TEXT NOT NULL,                 -- 'x86_64', 'aarch64'
    toolchains      JSONB NOT NULL DEFAULT '{}'::jsonb, -- Detected compiler/toolchain map
    env_quirks      TEXT,                          -- Free-form user notes on machine quirks
    registered_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_box_hostname ON box(hostname);

-- ============================================================================
-- 3. Work Items & History
-- ============================================================================
CREATE TABLE item (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    repo_id             UUID NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    item_key            TEXT NOT NULL,             -- e.g. 'ANA-1', 'MOD-6', 'TOOL-2'
    title               TEXT NOT NULL,
    body                TEXT NOT NULL DEFAULT '',  -- Current head specification
    status              TEXT NOT NULL DEFAULT 'open', -- 'open', 'in_progress', 'blocked', 'done', 'closed'
    version             INTEGER NOT NULL DEFAULT 1,   -- Optimistic locking counter
    created_by_box_id   UUID REFERENCES box(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_repo_item_key UNIQUE (repo_id, item_key),
    CONSTRAINT chk_item_status CHECK (status IN ('open', 'in_progress', 'blocked', 'done', 'closed'))
);

CREATE TABLE item_revision (
    item_id             UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    version             INTEGER NOT NULL,
    title               TEXT NOT NULL,
    body                TEXT NOT NULL,
    author_box_id       UUID REFERENCES box(id) ON DELETE SET NULL,
    reason              TEXT,                      -- e.g. 'initial_creation', 'plan_refinement', 'divergence_resolution'
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (item_id, version)
);

CREATE INDEX idx_item_repo_status ON item(repo_id, status);
CREATE INDEX idx_item_updated_at ON item(updated_at);

-- ============================================================================
-- 4. Directed Item Dependency & Lineage Graph (Explicit blocked_by)
-- ============================================================================
CREATE TABLE item_link (
    from_item_id    UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    to_item_id      UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    kind            TEXT NOT NULL,                 -- 'blocked_by', 'origin', 'relates', 'supersedes'
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (from_item_id, to_item_id, kind),
    CONSTRAINT chk_no_self_link CHECK (from_item_id <> to_item_id),
    CONSTRAINT chk_link_kind CHECK (kind IN ('blocked_by', 'origin', 'relates', 'supersedes'))
);

CREATE INDEX idx_item_link_to ON item_link(to_item_id);

-- ============================================================================
-- 5. Multi-Agent Execution Pipelines & Phase Steps
-- ============================================================================
CREATE TABLE run (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    item_id         UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    box_id          UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    status          TEXT NOT NULL DEFAULT 'running', -- 'running', 'completed', 'failed', 'cancelled'
    started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at    TIMESTAMPTZ,
    CONSTRAINT chk_run_status CHECK (status IN ('running', 'completed', 'failed', 'cancelled'))
);

CREATE INDEX idx_run_item ON run(item_id, started_at DESC);

CREATE TABLE run_step (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES run(id) ON DELETE CASCADE,
    step_number     INTEGER NOT NULL,              -- 1, 2, 3...
    phase           TEXT NOT NULL,                 -- 'prd', 'plan', 'implement', 'review', 'interactive'
    agent_name      TEXT NOT NULL,                 -- 'claude', 'agy'
    agent_model     TEXT NOT NULL,                 -- 'gemini-3.8-flash', 'claude-3-5-sonnet', etc.
    status          TEXT NOT NULL DEFAULT 'running', -- 'running', 'completed', 'failed', 'cancelled'
    exit_code       INTEGER,
    prompt_digest   TEXT NOT NULL,                 -- SHA-256 hash of assembled prompt
    commit_hash     TEXT,                          -- Git commit targeted or produced by this step
    transcript_ref  TEXT,                          -- Generic URI ('file://...', 'attachment://...', 's3://...')
    started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at    TIMESTAMPTZ,
    CONSTRAINT uq_run_step_number UNIQUE (run_id, step_number),
    CONSTRAINT chk_step_status CHECK (status IN ('running', 'completed', 'failed', 'cancelled'))
);

CREATE INDEX idx_run_step_run ON run_step(run_id, step_number);

-- ============================================================================
-- 6. Macro Phase Documents (PRD, Plan, Review, Summary)
-- ============================================================================
CREATE TABLE item_document (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    item_id         UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    run_step_id     UUID REFERENCES run_step(id) ON DELETE SET NULL,
    kind            TEXT NOT NULL,                 -- 'prd', 'plan', 'review', 'summary'
    version         INTEGER NOT NULL DEFAULT 1,    -- Iteration/retry version
    title           TEXT NOT NULL,
    content         TEXT NOT NULL,                 -- Full markdown text
    char_count      INTEGER NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT uq_item_doc_version UNIQUE (item_id, kind, version),
    CONSTRAINT chk_item_doc_kind CHECK (kind IN ('prd', 'plan', 'review', 'summary'))
);

CREATE INDEX idx_item_doc_item ON item_document(item_id, kind, version DESC);

-- ============================================================================
-- 7. Atomic Repository Facts & Findings (Artifacts)
-- ============================================================================
CREATE TABLE artifact (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    repo_id         UUID NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    item_id         UUID REFERENCES item(id) ON DELETE CASCADE,      -- Nullable for global repo facts
    run_step_id     UUID REFERENCES run_step(id) ON DELETE SET NULL,
    sequence_num    INTEGER NOT NULL,              -- Monotonic sequence within item/repo
    headline        TEXT NOT NULL,                 -- One-line summary of the fact
    content         TEXT NOT NULL DEFAULT '',      -- Detailed context, reproduction, code excerpt
    importance      INTEGER NOT NULL DEFAULT 5,    -- 1 (minor note) to 10 (critical architectural invariant)
    commit_hash     TEXT,                          -- Git commit where this fact was confirmed
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT chk_artifact_importance CHECK (importance BETWEEN 1 AND 10)
);

CREATE INDEX idx_artifact_repo_importance ON artifact(repo_id, importance DESC);
CREATE INDEX idx_artifact_item ON artifact(item_id, sequence_num);

-- ============================================================================
-- 8. Fact-to-Fact Relationships (Knowledge Graph)
-- ============================================================================
CREATE TABLE artifact_link (
    from_artifact_id UUID NOT NULL REFERENCES artifact(id) ON DELETE CASCADE,
    to_artifact_id   UUID NOT NULL REFERENCES artifact(id) ON DELETE CASCADE,
    kind             TEXT NOT NULL,                 -- 'supports', 'refines', 'contradicts', 'relates_to'
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (from_artifact_id, to_artifact_id, kind),
    CONSTRAINT chk_no_self_artifact_link CHECK (from_artifact_id <> to_artifact_id),
    CONSTRAINT chk_artifact_link_kind CHECK (kind IN ('supports', 'refines', 'contradicts', 'relates_to'))
);

CREATE INDEX idx_artifact_link_to ON artifact_link(to_artifact_id);

-- ============================================================================
-- 9. Sync State per Box and Repo
-- ============================================================================
CREATE TABLE sync_state (
    box_id          UUID NOT NULL REFERENCES box(id) ON DELETE CASCADE,
    repo_id         UUID NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    last_synced_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    sync_cursor     TIMESTAMPTZ NOT NULL DEFAULT '1970-01-01 00:00:00+00',
    status          TEXT NOT NULL DEFAULT 'clean', -- 'clean', 'diverged', 'syncing', 'error'
    last_error      TEXT,
    PRIMARY KEY (box_id, repo_id),
    CONSTRAINT chk_sync_status CHECK (status IN ('clean', 'diverged', 'syncing', 'error'))
);

-- ============================================================================
-- 10. External Issue Tracker Mappings (Pluggable, Pending ANA-6)
-- ============================================================================
CREATE TABLE external_issue_mapping (
    item_id         UUID NOT NULL REFERENCES item(id) ON DELETE CASCADE,
    provider        TEXT NOT NULL,                 -- 'onedev', 'github', 'linear'
    external_id     TEXT NOT NULL,                 -- Issue number or external UUID
    external_url    TEXT,
    last_synced_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (item_id, provider)
);
```

### 4.1 Recursive CTE Example: Prioritized Fact & Document Injection for Prompts

When preparing the initial prompt for an item, `htui` walks 1–2 hops outward across `item_link` to collect
prerequisite items (`kind IN ('blocked_by', 'origin')`), fetches their active `summary` documents, and selects
high-importance atomic facts (`importance >= 7`):

```sql
-- Walk 1-2 hops outward to find upstream prerequisites of :target_item_id
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
    
    -- Recursive: 2nd hop prerequisites
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
-- 1. High-importance atomic facts from upstream items and global repo
SELECT 
    'fact' AS entry_type,
    i.item_key,
    a.headline,
    a.content,
    a.importance,
    a.commit_hash
FROM upstream_graph ug
JOIN item i ON i.id = ug.to_item_id
JOIN artifact a ON a.item_id = i.id
WHERE a.importance >= 7

UNION ALL

-- 2. Distilled summaries from upstream items
SELECT 
    'summary' AS entry_type,
    i.item_key,
    d.title AS headline,
    d.content,
    10 AS importance,
    NULL AS commit_hash
FROM upstream_graph ug
JOIN item i ON i.id = ug.to_item_id
JOIN item_document d ON d.item_id = i.id AND d.kind = 'summary'
WHERE d.version = (
    SELECT MAX(version) FROM item_document WHERE item_id = i.id AND kind = 'summary'
)
ORDER BY importance DESC;
```

This single query bounds context consumption by retrieving exactly what the agent needs without
reading raw logs or requiring an external graph database.

---

## 5. The Thin Per-Repo `items.json` Specification

To ensure git repositories remain completely merge-friendly across branches and boxes, `items.json`
is strictly minimal. It contains only identity, key, title, and status.

### 5.1 Schema Format

File path: `<repo_root>/items.json`

```json
{
  "$schema": "https://json.schemastore.org/htui-items-v1.json",
  "version": 1,
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
    }
  ]
}
```

### 5.2 Formatting & Merge Laws

1. **Deterministic Ordering:** Items are sorted naturally by `key` (e.g. `ANA-1`, `ANA-2`, `MOD-1`, `MOD-10`).
2. **Compact Line Formatting:** Each item object is serialized cleanly. Diff-friendly trailing commas
   are maintained where possible.
3. **No Body, No Narrative:** All detailed prose, acceptance criteria, and run logs are strictly
   excluded. Two developers editing different items on different branches produce zero merge conflicts.
4. **Offline Cold-Start:** When a developer clones a repository on a machine with no network connection
   to Postgres, `htui` opens immediately in **offline read mode**, parsing `items.json` to display the
   backlog and key coordinates.

---

## 6. Box Registry and Prompt Injection Contract

### 6.1 Box Hardware & Toolchain Probe

On startup, `htui` executes an automatic, non-blocking hardware and toolchain probe (`MOD-7`).
The results are saved to Postgres in `box.toolchains` and cached locally in `~/.config/htui/box.json`:

```json
{
  "hostname": "DESKTOP-RIG",
  "os": {
    "family": "windows",
    "version": "10.0.26100",
    "arch": "x86_64"
  },
  "compilers": {
    "rustc": "1.82.0",
    "cargo": "1.82.0",
    "gcc": "14.2.0 (MinGW-W64)",
    "clang": null,
    "cl": "19.41.34120 (MSVC v143)"
  },
  "build_tools": {
    "cmake": "3.30.2",
    "ninja": "1.12.1",
    "vcpkg": "present (D:/vcpkg)"
  },
  "shells": {
    "pwsh": "7.4.5",
    "powershell": "5.1.26100",
    "bash": "present (MSYS2 - fork penalty detected)"
  },
  "env_quirks": "Windows 11 box. MinGW GCC 14.2 is default C++ compiler. Bash subprocesses suffer 0xC0000142 fork stalls; always invoke pwsh .ps1 scripts."
}
```

### 6.2 Prompt Injection Contract

When `htui` launches an agent (`claude`, `agy`), the prompt builder prefixes the task prompt with
the host box context:

```markdown
<HOST_ENVIRONMENT>
Hostname: DESKTOP-RIG (windows / x86_64)
Compilers: rustc 1.82.0, gcc 14.2.0 (MinGW), cl 19.41 (MSVC)
Tools: cmake 3.30.2, ninja 1.12.1, vcpkg at D:/vcpkg
Default Shell: pwsh 7.4.5
Machine Quirks: MinGW GCC 14.2 default. Bash fork stalls detected; use pwsh for scripts.
</HOST_ENVIRONMENT>
```

This prevents the agent from making invalid assumptions about available tools, file path conventions,
or compiler arguments.

---

## 7. Transcript Privacy and the Split Model

To prevent secret leakage while preserving both human readability and deep diagnostic capability,
execution transcripts are split into two distinct destinations:

```
[Agent Execution Finished]
          |
          v
[Local Scrubbing Pipeline (In-Memory on Host Box)]
  - Redact API keys: sk-ant-*, ghp_*, eyJ*
  - Redact private keys & certificates
  - Redact environment variables loaded from Infisical / OS
  - Hash raw input (SHA-256 prompt_digest)
          |
          +-----------------------------------+
          |                                   |
          v                                   v
[Destination 1: Step Summary]       [Destination 2: Full Debug Log]
  - Distilled narrative               - Complete raw JSON-RPC / CLI stream
  - Decision outcomes & commit hashes - Compressed and uploaded as attachment
  - Posted as Issue Comment           - Referenced as generic URI (`transcript_ref`)
```

### 7.1 Local Secret Scrubbing Pipeline

The scrubber runs in Rust prior to any network call:
1. **Entropy & Pattern Redaction:** High-precision regex masks replace known key formats
   (`sk-ant-[a-zA-Z0-9_\-]{40,}`, `Bearer [a-zA-Z0-9\-._~+/]+=*`, `ghp_[a-zA-Z0-9]{36}`) with `[REDACTED_SECRET]`.
2. **Dynamic Inventory Redaction:** All environment variables registered in the local session or
   fetched via secret manager (Infisical, `ANA-7`) are added to an exact-match redaction dictionary.
3. **Audit Verification:** If an unmasked secret pattern is detected, upload is blocked, and the step
   is flagged with `status = 'failed'` and an error alert in the TUI.

---

## 8. Pluggable Issue Tracker Mapping (`IssueSync`)

To keep `htui` adaptable and avoid hard coupling to a single issue tracker, the issue
synchronization engine is defined behind a Rust trait:

```rust
#[async_trait]
pub trait IssueSync: Send + Sync {
    /// Synchronize work item status and title to remote issue.
    async fn sync_item(&self, item: &Item) -> Result<ExternalMapping, SyncError>;

    /// Post human-readable step summary as a comment.
    async fn post_summary_comment(&self, external_id: &str, summary: &str) -> Result<(), SyncError>;

    /// Upload scrubbed debug transcript as an issue attachment.
    async fn upload_transcript(
        &self, 
        external_id: &str, 
        filename: &str, 
        data: &[u8]
    ) -> Result<String, SyncError>;
}
```

*Note: Final decision on whether OneDev remains mandatory, optional, or replaced by local/Postgres
discussion storage is formally evaluated in `ANA-6`.*

---

## 9. Conflict Resolution and Divergence Detection

When two development boxes make edits to the same item while disconnected, silent overwrites are
forbidden:

1. **Optimistic Locking:** Every `item` row carries a `version` integer and `updated_at` timestamp.
2. **Field-Level Last-Writer-Wins (LWW):** For non-critical metadata (e.g. `status`), the update with
   the higher `updated_at` timestamp is accepted.
3. **Application-Managed Revisions & 3-Way Diffing:**
   * On every edit, `htui` increments `item.version` and inserts a snapshot into `item_revision`.
   * If Box A updates `item.body` from version 3 to 4, while Box B simultaneously updates `item.body`
     from version 3 to 4 with different text, Box B's push is rejected.
   * `sync_state.status` is marked `'diverged'`.
   * The TUI displays a **Divergence Alert Modal**, pulling the common ancestor (`version = 3`) from
     `item_revision` alongside Box A's and Box B's text to present a clear 3-way merge interface.

---

## 10. Phasing & Spawned Implementation Modules

| Module | Title | Scope & Deliverables |
|---|---|---|
| **MOD-6** | **Item store: `items.json` + Postgres** | `sqlx` migrations for the DDL schema above; `ItemStore` trait implementation in Rust; local `items.json` parser, serializer, and deterministic sorter; offline queueing engine; `item_revision` application management. |
| **MOD-7** | **Box registry + prompt injection** | Hardware/toolchain probe implementation in Rust; box profile cache (`~/.config/htui/box.json`); DB registration; prompt builder injection module. |
| **MOD-5** | **Issue tracker sync** | `IssueSync` trait implementation (OneDev / generic REST); background sync worker; secret scrubbing pipeline; scrubbed transcript uploader. Blocked on `ANA-6`. |

---

## 11. Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Postgres unreachable during offline travel | TUI fails to start | Local `items.json` provides an immediate offline snapshot. `ItemStore` operates in local cached mode. |
| Secret leaked into transcript attachment | High security risk | Scrubbing is executed locally in Rust before network transmission; fail-closed upload policy. |
| Concurrent git edits on `items.json` cause merge conflicts | Developer friction | `items.json` is stripped of all bodies and comments, containing only UUID, key, title, and status sorted deterministically. |
| Prompt tokens exhausted by deep dependency trees | High token burn | Neighbor search is capped at 1–2 hops via recursive SQL CTE; only facts with `importance >= 7` and active summaries are injected. |
| Fact obsolescence across repo commits | Stale context | Every `artifact` record carries a `commit_hash`, allowing `htui` to detect and flag facts verified on outdated git commits. |

---

## 12. Validation Criteria

1. **Schema Migration Integrity:** Postgres schema migrations run forward and backward cleanly via `sqlx-cli`.
2. **Recursive Traversal Performance:** 2-hop CTE query executes in < 5 ms across 10,000 synthetic items, links, and facts.
3. **`items.json` Parity:** Round-trip parsing and serialization of `items.json` produces byte-identical output with zero drift.
4. **Box Probe Correctness:** `box` probe detects host compilers and tools on both Windows (MSVC/MinGW) and Linux systems without hanging.
5. **Secret Redaction Test Suite:** 100% of synthetic test secrets (Anthropic keys, bearer tokens, AWS secrets) are redacted before transcript upload.

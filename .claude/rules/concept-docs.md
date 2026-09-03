---
paths:
  - "CONCEPTS.md"
  - "**/CONCEPTS.md"
  - ".claude/rules/concept-docs.md"
---

# Concept Docs Rule

Governs `CONCEPTS.md` at the workspace root and in each active repo. Read this before creating or
editing one. Adopted by workspace MOD-11 from `docs/ANA-11.md` §4–§5.

## What a `CONCEPTS.md` is

The standing-present tense of the doc surface. `HANDOFF.md` is the future (what is still open),
`DECISIONS.md` the past (what was resolved), `CONCEPTS.md` the present: the statements that are true
right now and stay true between commits.

It exists because nothing else can hold them. graphify's `graph.json` has no prose field at all — it
indexes files, symbols, edges and communities, so *why the RHI seam exists* is unanswerable there by
construction, not by omission. `README.md` says what a thing does; `CLAUDE.md` tells the agent what
to do; `docs/ANA-*.md` argues a question once and freezes at its verdict. The standing conclusion
had no home before this file.

## What it owns

Intent that no extractor can produce:

- what the project or library **is**, and what it deliberately is **not**;
- **invariants** — the properties that must hold, and what breaks if they do not;
- **rejected alternatives** — what was considered and why it lost;
- the **laws** that bind consumers (and a link to wherever they are enforced);
- the **why** behind a seam, a split, or a toolchain choice.

One or two sentences per statement, then a link to the authority. `CONCEPTS.md` states the standing
conclusion; it never re-argues it.

## What it must never contain

| Forbidden | Because |
|---|---|
| File listings, directory trees | graphify produces these, correctly, on demand |
| Symbol tables, API signatures, call edges | same — and they are stale within a week |
| Target-dependency tables | the CMake target graph is extracted, not written |
| Live values — versions, baselines, port REFs, pinned hashes | those belong in `HANDOFF.md` (workflow-docs rule 6). State the *protocol* here, keep the *current numbers* there |
| Build/run instructions | `CLAUDE.md` (agent-facing) or `README.md` (human-facing) |
| Anything already stated in this repo's `README.md` / `CLAUDE.md` | **link it instead.** Two copies of one fact is how this file rots — the failure is documented (go-github#397) and it is the single most likely way this layer dies |

Mechanically: a markdown table whose header row carries `file`, `symbol`, `target` or `depends` is a
violation.

## Shape

1. **Header, mandatory, first lines, both parse-errors if absent:**

   ```
   **Owner:** <name>
   **Reviewed:** YYYY-MM-DD
   ```

   A document without an owner goes stale and no one notices; the header is the only intervention in
   the literature with an operating track record (SWE-at-Google ch. 10).

2. **Hard size cap: 4 096 bytes** for the workspace-root file, **6 144 bytes** for a repo-level one,
   **4 096 bytes** for a nested one (clause 3).
   Enforce size, not structure — template rigour has no measured effect on comprehension
   (Ernst & Robillard), while length does. Hitting the cap is a signal, not a nuisance: it means
   either a statement belongs one level down, or the deferred per-source-subdir tier's trigger is
   half-met (`docs/ANA-11.md` §5.2). **Never widen the cap to fit more prose.** A **frozen** repo's
   file is exempt: it is a historical record that may not be edited down, and the freeze banner it
   gains (workspace MOD-14) can only push it further over. Exemption applies to frozen repos alone —
   an active file never buys headroom by claiming to be history.

3. **Repo-level files disambiguate by H1**: `# CONCEPTS - vfs`. **A nested file — one level below
   repo level, e.g. `engine/din/vfs/CONCEPTS.md`** — disambiguates the same way but carries the full
   nested path: `# CONCEPTS - engine/din/vfs`. Its cap is **4 096 bytes**, the workspace-root figure
   rather than the repo-level 6 144: a nested file states less by construction, and a widened cap
   here would be clause 2's "never widen the cap" broken by a side door.

   The tier was deferred pending the two-part trigger in `docs/ANA-11.md` §5.2 (a session
   demonstrably re-derives an invariant the repo file was too coarse to state, **and** that file has
   hit its cap). Workspace **MOD-14** opened it by maintainer override *ahead of* that trigger
   firing: three libraries folded into `engine/din/` (engine MOD-33) left their standing intent
   homeless — each origin file was already over its own cap, and the fold deleted the repo file's
   audience, which the two-part trigger did not anticipate and cannot fire on. Recorded at
   `docs/decisions/mod/mod-14.md`. The override is scoped to that one fold: it does not repeal the
   trigger for any further nesting, and nothing nests below one level without a fresh trigger of the
   same shape.

## Update trigger

Edited **at close-out only** — in the same commit as the `DECISIONS.md` index line — when:

- an ANA verdict lands that changes a standing conclusion;
- a repo changes lifecycle status (active / frozen / donor);
- a workspace-level law changes.

**Never on a code change.** That is the whole cost model: this file is edited a handful of times a
month against a workspace running hundreds of commits in the same period. `**Reviewed:**` moves on
those edits and on nothing else — there is no periodic review cadence, because an unenforced cadence
is the rot this rule exists to prevent.

## Distribution

- **`CONCEPTS.md` never joins any sync list.** Each one is repo-specific by definition, and the
  sync's mirror semantics would overwrite every sub-repo's concepts with the workspace's.
- **This rule file does sync**, like `workflow-docs.md`, so every repo carries the same law.

## Enforcement status

The caps, the header and the forbidden-content regex are **specified here and not yet automated**.
`validate-concept-docs` (workspace **TOOL-5**, `docs/ANA-11.md` §6.5) is the item that machine-checks
them, and it is deliberately non-blocking at close-out. Until it lands, this rule is enforced by the
reviewer gate and by whoever writes the file.

Note also what does *not* check these files today: `validate-workflow-docs`'s `paths:` scope covers
`HANDOFF.md`, `DECISIONS.md`, `docs/decisions/**` and `docs/ANA-*.md` — **not** `CONCEPTS.md`. Links
written here are unverified until TOOL-5.

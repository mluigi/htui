# ANA-36 - Further MCP tools for agents (concluded, 2026-10-18)

Full analysis: `docs/ANA-36.md` (the shipped surface, other harnesses, transcript evidence, the 36-tool
catalogue, options, rejected ideas). Origin: the 2026-10-15 requirements review,
`docs/requirements-review-2026-10-15.md` (`R-MCP-2`). Addresses `R-MCP-2`, `R-MCP-1`, `R-ID-6`; touches
`R-MCP-3`, `R-MCP-4`, `R-ENT-8`, `R-ENT-9`, `R-ENT-12`, `R-ENT-15`, `R-ID-5` and `R-ID-7` at their seams.

## Verdict

The maintainer accepted all thirteen answers of `docs/ANA-36.md` §8 ("all as recommended").

htui needs fewer new tools than the field ships, because its prompt and store already do much of what
other harnesses hand to the model: each step is a fresh session with a self-contained prompt
(`R-PRM-1`), and status, close-out and scheduling stay deterministic (`R-ID-6`, MOD-11 I-4). What agents
lack is being told about the tools they have, long commands that neither block nor force re-runs, a few
typed requests and proposals that wait for a person, and the reads MOD-112 already owns. Of 36
candidates, 11 are refuted (§10), 15 stay with the item that owns them (§7.2 hands each its design
input), and the 10 unowned survivors become seven items. Every new tool follows the fifteen rules of
§7.4 (scope from the token, propose versus dispose, authors marked, fenced writes, fail-closed scrub,
typed results, outline-first reads, handles with deadlines for long work, one tool-to-engine channel).

## The thirteen answers, in brief

1. **Catalogue:** 11 refuted, 15 left with their owners, 10 survivors grouped into seven items.
2. **Agent links:** a link of a kind a scheduler, status or subgraph reads (`blocked_by`,
   `supersedes`, `duplicates`, `parent`) is pending until a person accepts it; `relates`, `origin` and
   `implements` stay live as display. This reverses MOD-11 OQ-4 for those kinds.
3. **`requirement_cite`:** joins `R-MCP-2` as a pending `addresses`, `reserves` or `verifies` citation;
   `amends` and `withdraws` go through `requirement_propose` (MOD-118).
4. **Status requests:** `item_status` writes a typed request row a person accepts or rejects, shown on
   waiting-on-you and MOD-135's checklist; it never moves a status.
5. **Long commands:** `command_run` stays blocking; `command_start` / `command_wait` / `command_cancel`,
   a stored scrubbed log per command row, `command_output_read` and a `repo` argument are added. The log
   cap is the plan's.
6. **Results and presets:** pluggable parsers with the log read as fallback, a read-only view of the
   run's verify result, named presets with their own timeout instead of a `gate_run` tool; after MOD-152.
7. **Tool guidance:** `initialize.instructions` and a prompt section name every advertised tool, instead
   of a `session_info` tool; `COMMAND_QUEUE_TEXT` is reworded, which changes prompt digests.
8. **One tool-to-engine channel:** MOD-75 builds it first; MOD-27's consult reuses it.
9. **Cross-agent text in reads:** another agent's note returned by `item_read` is item work, marked with
   its author; unaccepted drafts never reach another run (`R-MCP-1` gains §9.1's sentence).
10. **Skill resources:** skill text stays inlined; a bound skill's resources are read on demand through
    MOD-113's read tool (`R-ID-5` amended, scoped to the step's bound skills).
11. **Same-item transcript tail:** MOD-112 part (1) gains a scrubbed transcript tail of a named step of
    the same item; other items' transcripts stay out.
12. **`document_patch` and attachment tools:** filed as wanted later; the attachment tools also wait on
    MOD-114 (3) and ANA-35.
13. **Requirement edits:** §9 accepted as written; applied to `docs/REQUIREMENTS.md` (`R-MCP-1`,
    `R-MCP-2` replaced, `R-MCP-3`, `R-MCP-4`, `R-ENT-9`, `R-ENT-15`, `R-ID-5`), with ANA-34 §10.11's
    coordinator-tools sentence folded into `R-MCP-2` so it is amended once.

## Spawned items

| Filed | From | Subject | Blocked on |
|---|---|---|---|
| MOD-151 | P1 | Tell agents about htui's MCP tools (server instructions, prompt section, typed results) | none |
| MOD-152 | P2 | Async command queue and stored command logs | none |
| MOD-153 | P3 | Structured test results and named command presets | MOD-152 |
| MOD-154 | P4 | `item_status` as a typed request | ANA-35 (status enum), MOD-135 |
| MOD-155 | P5 | Agent link and citation proposals wait for a person | ANA-35 (link kinds, `W-` keys) |
| MOD-156 | P6 | `document_patch` tool (wanted later) | none |
| MOD-157 | P7 | Attachment tools for agents (wanted later) | MOD-114 (3), ANA-35 |

ANA-35 was concluded on the unmerged branch `hr/ANA-35`, which is not reachable from this branch; its
spawned items (MOD-144..150) are not filed here, so the blockers above name ANA-35. When that branch
merges, MOD-154 points at its status item and MOD-155 and MOD-157 at its link-kinds item.

Effects on open items (one pointer each in `HANDOFF.md`): MOD-112, MOD-75, MOD-27, MOD-135, MOD-136,
MOD-118, MOD-119, MOD-121, MOD-113, MOD-110, MOD-114, ANA-38 and ANA-39 carry the §7.2 design input (or,
for ANA-39, §10's code-search refusal); MOD-75 builds the one tool-to-engine channel and MOD-27's consult
reuses it (Q8); MOD-109 gains MOD-152 and MOD-153 as consumers of its limit and preset forms. MOD-140
lives on the unmerged ANA-33 branch and carries no pointer here.

## Commits

`4e78d58d` (draft), the conclusion commit that adds this file, and the follow-up commit that applies
§9 to `docs/REQUIREMENTS.md`.

## Not verified

The transcript figures in `docs/ANA-36.md` §4 are call counts, not durations or cost; the ANA-33
references read `origin/hr/ANA-33`, which is unmerged; the ANA-35 dependencies are named by the ANA, not
by its spawned items, because `hr/ANA-35` is not reachable here.

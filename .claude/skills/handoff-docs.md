# HANDOFF workflow — how the two commands fit together

Orientation for the `/handoff-*` command set: what each one does, which to reach for, and where the
boundaries are. This file **explains**; it never legislates. The law lives in
`.claude/rules/workflow-docs.md` (item prefixes, file roles, lifecycle rules) and in each skill's own
`SKILL.md`. On any conflict, those win.

Same surface in every repo. This file, `.claude/rules/workflow-docs.md`, and the two skill
directories are copied from the **dingine workspace** by `sync-workflow-surface.sh`/`.ps1` — edit the
workspace copy, never a sub-repo copy (see *Distribution* below).

## The two commands

| Command | Produces | Touches lifecycle files? |
|---|---|---|
| `/handoff-add <description>` | A new `HANDOFF.md` checklist item, ID minted | Yes — `HANDOFF.md` only |
| `/handoff-run <ITEM-ID>\|next` | The whole lifecycle: select → route → artifacts → implementation → close-out | Yes — this is the only command with full authority |

Both are interactive: a maintainer is present, and every gate below is a decision they make. Nothing
in this surface runs unattended.

## The normal path

```
/handoff-add "..."      ->  HANDOFF.md item
        |
/handoff-run <ID>|next  ->  `next`: selection block          [maintainer confirms / breaks ties]
        |                   route verdict (ANA | PRD | plan)  [maintainer confirms]
        |                   artifacts: docs/ANA-N.md  or  .claude/prds/*.prd.md + .claude/plans/*.plan.md
        |                   CONFIRM gate on the plan
        |                   implementation: code-architect -> implementer fan-out -> reviewer gate
        |                   close-out: HANDOFF line deleted, write-up file + index line, validator
        v
     item done
```

## `/handoff-add` — open an item

Turns one line of description into a well-formed item: picks the prefix (`ANA`/`MOD`/`NEXT`/`VAL`/
`TOOL`/`CLEAN`), mints the next ID by running
`.claude/skills/handoff-run/scripts/next-item-id.sh --prefix <PREFIX>` (macOS/Linux) or
`next-item-id.ps1 -Prefix <PREFIX>` (Windows) (owned-ID method; IDs are
per-repo and never reused), files it under the right section, adds origin / blocked-on cross-links,
recounts the summary table, and leaves the validator green. It shows a verdict and waits before
writing.

Use it instead of hand-editing `HANDOFF.md` — the ID mint and the recount are exactly the steps that
go wrong by hand. The mint script is runnable on its own in any repo (`-All` for a per-prefix
report) and **blocks on a non-zero exit**: an unparseable index line or a leftover legacy write-up
section means the ID space cannot be trusted, and minting over it is how an ID gets reused.

## `/handoff-run` — the spine

One command per item lifecycle. Five things about it matter in daily use:

1. **You need not name the item.** `/handoff-run next` scans the current repo's `HANDOFF.md`, drops what is blocked or
   owned elsewhere, and ranks the rest: in-flight work first (a landed phase, a plan or PRD artifact named for the item),
   then whatever unblocks other open items, then section order (Next features → Analyses → Deferred backlog →
   validation → tooling). It prints what it picked, what came second, and what was ineligible. **A tie that only
   file order would break is a question, not a guess** — the maintainer is present, so an unresolved tie is asked,
   never coin-flipped. Rules in `handoff-run/references/selection.md`.
2. **Routing is mechanical.** `ANA-N` that asks a real question → ANA path (research doc, never
   implementation). Anything else scores four criteria — cross-repo reach, new public API surface,
   unresolved design questions in the item text, breadth over ~10 files — and **two or more fired
   means PRD path**, otherwise plan path. An `ANA-N` item that is really implementation work is a
   *prefix mismatch*: it gets re-minted as `MOD-N` rather than routed as research.
3. **The gates never disappear.** Selection confirm (`next` only), route confirm, `plan` CONFIRM, PRD open
   questions, and deliberate push are maintainer decisions in every run.
4. **The reviewer gate always runs last**, over the changes, resolved from the repo's
   `.claude/workflow-config.json` key `reviewer`. That file is per-repo and deliberately outside the
   synced surface — the sync script never owns or overwrites it.
5. **Close-out is bookkeeping, then proof.** Phase note for a milestone of multi-phase work, full
   close-out (HANDOFF line deleted, write-up file + index line, summary recount) when the item is done, then
   `validate-workflow-docs.sh`/`.ps1`. A red validator blocks the done-report.

`--dry-run` prints the routing verdict and stops (with `next`, the selection block too). Cheap, and the fastest way
to see how an item will be treated before committing to it.

## Artifacts, and where they live

| Artifact | Path | Written by |
|---|---|---|
| Open items | `HANDOFF.md` | `/handoff-add`, `/handoff-run` |
| Archive index (one line per item) | `DECISIONS.md` | `/handoff-run` close-out |
| Archived write-up (one file per item) | `docs/decisions/<prefix>/<prefix>-N.md` | `/handoff-run` close-out |
| Analysis / design doc | `docs/ANA-N.md` | `/handoff-run` ANA path |
| PRD | `.claude/prds/<name>.prd.md` | `/handoff-run` PRD path |
| Plan | `.claude/plans/<name>.plan.md` | `/handoff-run` PRD and plan paths |

Per-repo IDs: `MOD-1` here and `MOD-1` in another repo are unrelated items. Always name the repo when
referring to an ID outside its own `HANDOFF.md` ("engine MOD-7", "workspace MOD-5").

## Enforcement and distribution

- **Validator** — `handoff-run/scripts/validate-workflow-docs.sh`/`.ps1` checks the structural rules
  (sections, ID reuse, summary counts, cross-links, status-line cap, index/write-up-file agreement).
  Run it any time; it is cheap.
- **Commit hooks** — installed by `handoff-run/scripts/install-workflow-hooks.sh`/`.ps1`. `pre-commit`
  validates the whole repo when `HANDOFF.md`/`DECISIONS.md`/`docs/decisions/**`/`docs/ANA-*` is staged, but
  blocks only on errors in the **staged** files (`--scope-paths`): a pre-existing finding in a file this
  commit does not touch prints as `[WARNING] ... (pre-existing ...)` and still fails a direct validator run.
  Human hatch: `WORKFLOW_SKIP_HOOK=1` (not `--no-verify` — a separate PreToolUse hook hard-blocks that flag
  for agent sessions). In sub-repos it additionally blocks staged
  surface files that differ from the workspace source — committing sync results passes, local
  divergence blocks.
- **Sync** — `handoff-run/scripts/sync-workflow-surface.sh`/`.ps1` mirrors the surface from the workspace
  into the sub-repos; `-Check` reports drift without writing. The workspace `post-commit` hook runs it
  automatically when a commit touches surface source. `.workflow_version` marks the surface revision.

**So: to change how any of this behaves, edit the workspace copy and sync.** A sub-repo edit will be
overwritten by the next sync and blocked by that repo's pre-commit guard in the meantime.

## Choosing, in one paragraph

New work to record → `/handoff-add`. Work to actually do → `/handoff-run`, always; it is the only
command that can finish an item.

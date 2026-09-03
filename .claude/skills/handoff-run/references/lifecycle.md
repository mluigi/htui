# Lifecycle bookkeeping — /handoff-run

Operational procedure. **Law lives in `.claude/rules/workflow-docs.md`** — this file is the
ordered checklist the router executes, not a restatement of the rule. On any conflict, the
rule wins.

## P0 — Opening a new item (e.g. an ANA verdict spawns a MOD)

1. Mint the ID — run the script, do not derive it by eye:

   ```
   bash .claude/skills/handoff-run/scripts/next-item-id.sh --prefix <PREFIX>   # macOS/Linux
   pwsh .claude/skills/handoff-run/scripts/next-item-id.ps1 -Prefix <PREFIX>   # Windows
   ```

   It prints `max open` / `max archived` / `next`, and **non-zero exit blocks the mint** —
   an error means the ID space cannot be trusted, so fix the finding first (the report
   still shows the max, flagged `UNTRUSTED`, to help you fix it). `-All` reports every
   prefix; `-IdOnly` prints the bare ID for capture, and prints nothing at all when the
   space is untrustworthy.

   The rule it implements, which stays the definition: **owned IDs only** — max over open
   checklist lines (`^- \[ \] \*\*PREFIX-N`) in `HANDOFF.md` **plus** the archive: index
   lines in `DECISIONS.md` (`^- \*\*\[PREFIX-N\]`), cross-checked against the
   `docs/decisions/<prefix>/` directory listing. Then +1. Do **not** grep raw `PREFIX-N`
   mentions: cross-repo references (`<repo> PREFIX-N` mentions of other repos' items)
   pollute the count and inflate the ID. Two traps the index closes, both real: a
   compound header like `## MOD-18/19/20` hid two IDs from every `^## MOD-\d+` scan, and
   an index line that fails to parse drops its ID out of the count entirely — which is
   why both the script and the validator error on an unparseable line instead of skipping
   it. If `validate-workflow-docs.sh`/`.ps1` reports `decision-index`, do not mint until it
   is fixed.
2. Add the checklist line under the correct section
   (`- [ ] **PREFIX-N - Title.** body...`), noting origin if spawned ("from ANA-N").
3. Recount the summary table.
4. If the item originates from / blocks another repo: two-way cross-links per rule 5.

## P1 — Phase note (multi-phase item, not yet done)

1. Item's checklist line stays open; append `**Phase N landed (commit/date):** one-line
   note` to its body (rule 4).
2. No DECISIONS entry, no summary-table change, no status-line change.

## P2 — Close-out (item complete)

Ordered; do not interleave:

1. **Write-up**: create `docs/decisions/<prefix>/<prefix>-N.md` (lowercase dir, lowercase
   file — `MOD-7` → `docs/decisions/mod/mod-7.md`), opening with
   `# PREFIX-N - Title (status, YYYY-MM-DD)`. Full write-up: what was decided/built, why,
   commit hashes. For multi-phase items: one file covering all phases.
2. **DECISIONS.md**: prepend the index line at the **top** of the index
   (reverse-chronological):
   `- **[PREFIX-N](docs/decisions/<prefix>/<prefix>-N.md)** - Title (status, YYYY-MM-DD)`.
   Format law: `.claude/rules/workflow-docs.md` § Index line format.
3. **HANDOFF.md**: delete the item's checklist line. If the item still holds live
   coordinates other open work depends on (registry baselines, port REFs, pinned hashes),
   keep those in a one-line recap pointer near the status line (rule 6) — coordinates stay
   in HANDOFF, narrative goes to the write-up file.
4. **Summary table**: recount open items per prefix.
5. **Status line** (`**Current status (date):**`): update date, lead with this completion,
   and **cap the recap at the most recent ~2-3 completions** — drop the oldest mention
   rather than appending (rule 3).
6. **Cross-links**: any other item (this repo or another repo's HANDOFF) that pointed at
   this item as a blocker → update it to point at the item's write-up file / new owning
   item.
7. **Artifacts**: if the item had a PRD, set its milestone row complete; the plan file
   stays in `.claude/plans/` as record.
8. **Validate**: run
   `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` (macOS/Linux) or
   `pwsh .claude/skills/handoff-run/scripts/validate-workflow-docs.ps1` (Windows) from the
   repo root. Non-zero exit → fix findings before reporting the item done. The router
   never reports lifecycle-complete over a red validator. The same validator also runs
   automatically at commit time (`pre-commit` hook, installed by
   `install-workflow-hooks.sh`/`.ps1`, M3 — **on Windows install with the `.ps1`, which
   writes `pwsh` call sites into the hook; a bash-installed hook makes every workflow-doc
   commit hang instead of fail**) — but the hook blocks only on findings in the
   files that commit stages, so this manual whole-repo run is the one that must be green
   before the done-report. Pre-existing findings elsewhere show up in the hook output as
   `[WARNING] ... (pre-existing ...)`. Human escape hatch: `WORKFLOW_SKIP_HOOK=1`.

## P3 — Cross-repo pointers (rule 5 recap)

- Dependent entry notes: blocked on `<repo> <PREFIX-N>` (`<path>/HANDOFF.md`).
- Owning entry notes: originates from `<repo> <PREFIX-N>`.
- Never duplicate the other repo's item body; keep the pointer current instead.
- IDs are per-repo — always prefix with the repo name when referencing across repos.

## Commit & push

Bookkeeping edits ride with the work's commit(s) where possible, message per repo
convention. Push stays deliberate (the repo's `CLAUDE.md` push policy — workspace-level
where the repo is a submodule): push when the work was agreed/asked, never as an
automatic router side effect.

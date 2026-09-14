# TOOL-1 - next-item blocked-on regex counts only the first ID per phrase (concluded, 2026-09-14)

## Context
The `next-item.ps1` and `next-item.sh` scripts use a regex to extract blocked-on cross-links from `HANDOFF.md` body text. A bug caused comma-separated blockers (e.g. `Blocked on ANA-4, ANA-5`) to drop every ID after the first, which led to R2 dependent counts undercounting and wrapped blockers hiding entirely.

## Decision
Updated both `next-item.sh` and `next-item.ps1` to loop through consecutive IDs separated by commas immediately following the `blocked on` phrase.
1. Added `FOLLOW_PATTERN` in both scripts to capture chained IDs.
2. Modified `get_blocked_refs` / `Get-BlockedRefs` to extract the first matched ID and then run a secondary match-loop over the trailing text using `FOLLOW_PATTERN` to capture any comma-separated subsequent IDs, spanning across line wraps correctly.
3. Verified the partial phase-scoped rules carry over to all chained blockers.
4. Added test cases to the scripts' self-test functions covering two-ID blockers and wrapped blockers, asserting the correct extraction of all IDs and dependents counting.

## Hashes
- Plan: `.claude/plans/tool-1-next-item-regex.plan.md`
- Implemented and fact-checked under the `plan` path.

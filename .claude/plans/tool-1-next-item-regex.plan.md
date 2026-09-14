# Plan: TOOL-1 next-item blocked-on regex counts only the first ID per phrase

## Tasks

### 1. Fix `next-item.sh`
- In `get_blocked_refs`, instead of processing the body strictly line-by-line using `$US` as a separator, we need to handle line wraps. We can replace `$US` with a space to form a continuous string for matching, or we can handle continuation lines explicitly.
- To "repeat-match the ID list after the phrase", after a successful match of `BLOCKED_PATTERN`, we should check the string immediately following the match. If it starts with a comma (or comma and space, including line wraps), we extract the next ID using a supplementary regex, and continue this in a loop until no more IDs are chained.
- Ensure that `partial` phase scopes still correctly apply to all IDs extracted from a comma-separated list within a phase span.

### 2. Fix `next-item.ps1`
- In `Get-BlockedRefs`, similarly join `$Item.Body` into a single string (e.g., using ` -join ' '`) to allow matching across line wraps.
- Implement the same repeat-match logic: after finding the initial `blocked on ...` match, loop to find subsequent IDs separated by commas immediately following the previous match.
- Preserve the exact behavior of `Partial` phase span detection for all matched IDs.

### 3. Add test fixtures
- In both `next-item.ps1` (inside `Invoke-SelfTest`) and `next-item.sh` (inside `run_self_test`), add a fixture with a two-ID blocker (`Blocked on MOD-A, MOD-B`) and a wrapped case (`Blocked on\nMOD-C, MOD-D`).
- Assert that all IDs in these lists are correctly identified as blockers and their dependent counts (R2) are correctly incremented.

## Verification
- Run `WORKFLOW_ALLOW_SH_ON_WINDOWS=1 pwsh next-item.ps1 -SelfTest` to verify the `.ps1` twin.
- Run `bash next-item.sh --self-test` to verify the `.sh` twin.
- Ensure both self-tests pass and parity is maintained.

### Verified Claims
| Claim | Verdict | Evidence |
|---|---|---|
| `next-item.sh` processes body line-by-line using `$US` | Verified | `next-item.sh:195` |
| `next-item.ps1` processes body line-by-line using `$Item.Body` | Verified | `next-item.ps1:125` |
| `BLOCKED_PATTERN` only matches the first ID | Verified | `next-item.sh:48`, `next-item.ps1:124` |

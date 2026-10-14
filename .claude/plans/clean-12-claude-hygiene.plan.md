# Plan: CLEAN-12 — `.claude/` hygiene found by ANA-29

**Source**: `HANDOFF.md` CLEAN-12 (from `docs/ANA-29.md` §4.1) — routed as plan (criteria: C3 only; maintainer accepted)
**Complexity**: Small (no behaviour change; ~3 files plus a 20-file untrack)

## Summary

Three hygiene fixes. (1) Stop tracking the zero-byte `.claude/.headroom_wrap_settings.lock` and ignore it.
(2) Settle `.claude/skills/generated/`: tracked by `b5e481f3` yet listed as tool-owned/untracked by
`scripts/hr` — recommended resolution is **tool-owned** (untrack + ignore), see D1. (3) The hook-doc half of the
item is mostly already done by TOOL-9/TOOL-8: re-verify the two cited spots and make only the residual edit.

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Lock file is committed and ignored nowhere | true | `git ls-files .claude/.headroom_wrap_settings.lock` lists it; `git check-ignore -v` empty; `.gitignore` has no `headroom` line |
| `generated/` is committed, 20 files | true | `git ls-files .claude/skills/generated` → 20 `gortex-*/SKILL.md`, one commit `b5e481f3 feat: add gortex skills` |
| `scripts/hr` lists it as tool-owned | true, **line moved** | `TOOL_EXCLUDES` is now `scripts/hr:697` (item says `:692`) |
| Nothing reads the generated files | true | repo-wide grep: only `scripts/hr`, `docs/hr-sandbox.md:122,378`, `tests/hr/hr.bats:1616-1621`, `HANDOFF.md`, `docs/ANA-29.md` name the path — all about exclusion, none load a skill |
| `core.hooksPath` is unset / hook claim in `lifecycle.md:98-99` is stale | **false now** | TOOL-9 (`db997a65`, `a4f71c54`) committed `scripts/githooks/{pre-commit,post-commit,pre-commit.ps1}`; `lifecycle.md` (step 8, ~L98-106) already documents `scripts/githooks/`, `core.hooksPath`, `scripts/gate.sh`/`.ps1`. Sandbox `git config core.hooksPath` printed empty because the installer has not been run in this clone — a per-clone setting, not a doc error |
| `install-workflow-hooks.sh:22-23` relies on a settings-file hook that does not exist | **partly stale** | Header now carries the `core.hooksPath` paragraph (L16-19); L21-24 still say "a separate PreToolUse hook hard-blocks that flag" without naming it. The block now exists: G6 of the `htui-dev` mod (`docs/htui-dev.md:36,259`, `.claude/skills/htui-dev/hooks/register.ts:138`). Same wording in `install-workflow-hooks.ps1:21,219` |
| Tasks touch disjoint file sets | **false** | T1 and T2 both edit `.gitignore` → run serially (trivially small) |

## Decisions for the maintainer

- **D1 `generated/`: tool-owned (recommended) vs committed.** Tool-owned: the names embed Gortex community
  counts (`…-src-4-dirs-record`), so they change on every re-index; `scripts/hr` already treats them like
  `.serena/`, `.kiro/`, `graphify-out/` (all untracked); nothing loads them from git. Committed alternative:
  drop `/.claude/skills/generated/` from `TOOL_EXCLUDES` and its three doc/test mentions — but then a sandbox
  re-index dirties the tree. `.claude/skills/graphify` is a separate committed skill and is untouched either way.
- **D2 installer wording.** The `.sh`/`.ps1` header is part of the synced workflow surface, so keep it
  repo-neutral and add only a parenthetical pointer ("in htui: G6, `docs/htui-dev.md`"). Alternative: leave
  the header as is, since TOOL-9 already made it accurate. Recommended: the pointer.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Ignore rules | `.gitignore:7-8` | rooted `/path` entries, one per line |
| Tool-owned dirs | `scripts/hr:697` | `TOOL_EXCLUDES` is the clone-local mirror of what the repo should not track |
| Doc twin parity | `install-workflow-hooks.sh:5` | `.sh` and `.ps1` kept in lockstep |
| Tests | `tests/hr/hr.bats:1616` | exclude-list assertions (only touched if D1 flips to "committed") |

## Files to Change

| File | Action | Why |
|---|---|---|
| `.claude/.headroom_wrap_settings.lock` | DELETE (`git rm`) | tool lock file |
| `.gitignore` | UPDATE | add `/.claude/.headroom_wrap_settings.lock`, and `/.claude/skills/generated/` under D1=tool-owned |
| `.claude/skills/generated/*/SKILL.md` (20) | DELETE (`git rm`) | D1 = tool-owned |
| `.claude/skills/handoff-run/scripts/install-workflow-hooks.sh` | UPDATE (comment only) | name the G6 block (D2) |
| `.claude/skills/handoff-run/scripts/install-workflow-hooks.ps1` | UPDATE (comment only) | lockstep twin (L21 and L219 context) |
| `.claude/skills/handoff-run/references/lifecycle.md` | UPDATE only if re-read shows residue | already correct per the table; no edit expected |
| `HANDOFF.md`, `DECISIONS.md`, `docs/clean-12.md` | UPDATE/CREATE | P2 close-out per `lifecycle.md`; D1 goes in DECISIONS |

If D1 flips to "committed": instead edit `scripts/hr:697`, `docs/hr-sandbox.md:122,378`, `tests/hr/hr.bats:1616-1621`
(and nothing under `generated/`).

## Tasks

### Task 1: Untrack and ignore the lock file
- **Action**: `git rm --cached` is not enough for a lock that may be recreated — `git rm` it, add the rooted ignore line.
- **Mirror**: `.gitignore:7-8`
- **Validate**: `git ls-files .claude/.headroom_wrap_settings.lock` empty; `touch` it, `git status --short` stays clean.

### Task 2: Make `generated/` tool-owned (D1)
- **Action**: `git rm -r .claude/skills/generated`, add `/.claude/skills/generated/` to `.gitignore`.
- **Mirror**: `scripts/hr:697`
- **Validate**: `git ls-files .claude/skills/generated` empty; recreate a file under it, `git status --short` clean; `bats tests/hr/hr.bats` unaffected (no `scripts/hr` change).

### Task 3: Hook-doc residue (D2)
- **Action**: re-read `lifecycle.md` step 8 and the installer headers once more against `scripts/githooks/` and `docs/htui-dev.md`; add the G6 pointer to the `.sh` header and the matching `.ps1` comment. Comments only.
- **Mirror**: lockstep rule in the installer header.
- **Validate**: `bash -n install-workflow-hooks.sh`; `bash install-workflow-hooks.sh --self-test`; diff of the two files shows comment-only changes.

### Task 4: Close-out (P2)
- **Action**: `docs/clean-12.md` write-up, DECISIONS entry (D1), HANDOFF checkbox, summary-table recount, status line.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` exits 0.

## Validation

```bash
git ls-files .claude/.headroom_wrap_settings.lock .claude/skills/generated   # both empty
bash -n .claude/skills/handoff-run/scripts/install-workflow-hooks.sh
bash .claude/skills/handoff-run/scripts/install-workflow-hooks.sh --self-test
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

No Rust changed, so no cargo gate. Reviewer gate: `rust-reviewer` per `.claude/workflow-config.json` runs over the diff
(comment/ignore changes only; expected to be a short pass).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Untracking `generated/` surprises someone who relied on the committed copies | Low | Nothing loads them from git; Gortex regenerates them; recorded in DECISIONS |
| Installer comment edit drifts from the `.ps1` twin | Low | Edit both in one commit; `--self-test` |
| Merge conflict with branches that touch `generated/` | Low | Deleting a file the other side regenerates is a one-line resolve |
| Host `git collect` sees a 20-file delete as large | Low | Commit message states it is an untrack |

## Acceptance
- [ ] Lock file and `generated/` untracked and ignored
- [ ] D1 and D2 settled with the maintainer and recorded
- [ ] Installer headers (`.sh`/`.ps1`) and `lifecycle.md` match what the repo has now
- [ ] Validator exits 0; no behaviour change

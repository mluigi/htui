# CLEAN-12 - `.claude/` hygiene found by ANA-29 (done, 2026-10-14)

**Origin:** ANA-29 (`docs/ANA-29.md` §4.1).

**What was done.** Run in a sandbox on `hr/CLEAN-12`, routed as a plan (only C3 fired). Plan
`.claude/plans/clean-12-claude-hygiene.plan.md` (D1, D2). No Rust, no behaviour change, no cargo gate.

- **Untracked and ignored (`60d80e30`).** `.claude/.headroom_wrap_settings.lock` (zero-byte tool lock) and the
  20 `.claude/skills/generated/gortex-*/SKILL.md` files are `git rm`ed and ignored by rooted `.gitignore`
  entries. **D1: `generated/` is tool-owned.** The names embed Gortex community counts, so they change on every
  re-index; `scripts/hr` (`TOOL_EXCLUDES`, now `scripts/hr:697`, not `:692`) already treated the directory like
  `.serena/`, `.kiro/` and `graphify-out/`; nothing loads the files from git. `scripts/hr`, `docs/hr-sandbox.md`
  and `tests/hr/hr.bats` needed no change. `.claude/skills/graphify` stays committed.
- **Hook docs (`d72e81f2`).** Most of the item was already done by TOOL-9: `lifecycle.md` step 8 and the
  installer headers already describe `scripts/githooks/` and `core.hooksPath`, so `.claude/skills/handoff-run/references/lifecycle.md` was not edited.
  The one stale spot was the installer's "a separate PreToolUse hook hard-blocks that flag", which now names its
  source (G6 of the `htui-dev` mod, `docs/htui-dev.md`). **D2:** comment-only, `.sh` and `.ps1` in lockstep, and
  the hook text the installer generates is unchanged. `bash -n` clean, `--self-test` 16/16.

**Owed on the host.** `scripts/hr collect CLEAN-12`. The merge deletes the 20 files from the host working tree;
Gortex writes them again on its next index and the ignore rule keeps them out of `git status`.

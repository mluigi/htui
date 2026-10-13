# MOD-93 - `htui` Claude Code plugin: one skill, one mod, emitted from the binary (done, 2026-10-13)

**Requirements:** `R-ID-4`, `R-ID-6`, `R-STO-1`, `R-TUI-11`, `R-NF-1`.
**Origin:** ANA-29 (`docs/ANA-29.md` §6, options S4 and U4; `docs/decisions/ana/ana-29.md`).
**Artifacts:** PRD `.claude/prds/mod-93-htui-claude-plugin.prd.md` (M1 emit + skill, M2 mod),
plan `.claude/plans/mod-93-htui-claude-plugin.plan.md` (D1-D15, verified-claims table), blueprint
`.claude/plans/mod-93-htui-claude-plugin.blueprint.md` (amendments A-1..A-12, accepted). Routed
**PRD** (C2, C3, C4), ultracode for the implement phase.
**User guide:** `docs/htui-claude-plugin.md`; a see-also line in `docs/htui-cli.md`; README section.

## The problem

A person working in plain `claude` in another project could not see what htui was waiting on
without switching to the TUI, and the session there did not know htui existed, so it did inline
work that belonged in htui's queue. htui's MCP server is token-bound to the sessions htui launches,
so a plugin cannot reach it (ANA-29 §4.3). MOD-92's read-only `--json` verbs were the base; nothing
in Claude Code called them.

## Decisions taken with the maintainer

- **Build it now** (ANA-29 §12 Q3, answered by running MOD-93).
- **An `AbovePrompt` band, not a status line**: a status line would replace the user's own.
- **Default directory** under htui's local data directory; **`emit` refuses a non-empty target
  unless `--force`**; Windows behaviour of mods is a note for the host and MOD-16, not a blocker.
- Blueprint amendments: kit tests in `htui/tests/` (A-1); the `/htui` command hook never reads
  `next`, so `validate` reports it as answering its own command rather than as a gate (A-2); no
  pane-open state, the poll asks `$.ui.panes()` (A-4); the skill names `docs/htui-cli.md` and the
  clone URL without a web link (A-7).

## What shipped

- **`htui claude emit [--dir PATH] [--force]`** (`crates/htui/src/claude_emit.rs`): writes a local
  directory marketplace `htui` holding plugin `htui` from files embedded with `include_str!` (one
  table; a unit test walks `crates/htui/claude-plugin/` against it). The binary's version is
  stamped into `plugin.json` (serde rewrite) and `hooks/version.ts`; the source tree carries
  `0.0.0-source`. It prints `claude plugin marketplace add <dir>` and
  `claude plugin install htui@htui` and runs neither; it never touches `~/.claude` or a repo.
  - **Floor:** `claude --version` through `htui-agent`'s bounded `capture_version`; below 2.1.287
    the plugin is skill-only (no `hooks/`, `types/`, and no `types`/`userConfig` in the manifest);
    `claude` missing or silent writes the mod and says the version was not checked.
  - **`--force` replaces only what `emit` wrote:** every file under the target must be one of the
    table's paths (plus Claude Code's own `htui/.claude-plugin/types/**`); anything else, the
    plugin source tree (`0.0.0-source`), a symlink, a path with `..` or not UTF-8 is refused
    (exit 2) and left untouched. The swap stages into `<dir>.emit-<pid>`, moves the old copy aside,
    renames in, removes the old copy; every failed rename rolls back; the target is classified
    again right before the swap, after the version probe.
  - **Exits:** 2 refused, 1 failed; no `EmitExit` reaches Sentry (sentences carry home paths).
- **Skill `htui`** (`crates/htui/claude-plugin/htui/skills/htui/SKILL.md`): when to use it; `htui status|waiting --json` and
  `htui --search-items='<q>' --json`; the envelope, exit codes, "empty stdout is a failure",
  "ignore unknown fields and values"; reading `waiting` rows and search hits; hand-off rules (point
  the person at htui, never work an htui item inline, never write htui's store, never run
  `--index-items`).
- **The mod** (`crates/htui/claude-plugin/htui/hooks/register.tsx`, `types/index.d.ts`): an `AbovePrompt` band
  (`htui: N waiting on you`, plus a skew or error line) polled from `htui waiting --json` on
  `$.clock.every`; a `/htui` pane listing rows (reason, item, step, age, text); turnless
  `/htui status|waiting|search <q>`; a skew line when `htui --version` differs from the plugin's
  version; stand-down on `HTUI_SESSION` (MOD-91), where `/htui` is not registered and answers
  "stands down" if reached. `userConfig`: `panel` (`auto|command|off`), `poll_seconds` (5-3600,
  0 = 30). Reach is `$.process.run` (every call with `timeoutMs`), `$.clock`, `$.env.get`,
  `$.command`, `$.state`, `$.ui`; no `$.model`, `$.http`, `$.fs`, `$.session`. Failures become a
  visible line, never a throw; an answer outside the documented schema reads
  `htui: unreadable answer (exit N)`; htui's text goes through a control-character filter.
- **Gate** (`crates/htui/tests/claude_plugin.rs`, no feature gate): when `claude` ≥ 2.1.287 is on
  `PATH`, emits both variants into temp dirs, runs `claude plugin validate --strict --json` on
  marketplace and plugin (asserting the mod's `calls:` stay in the allowed nouns and there is no
  gating hook), runs `claude plugin test` on the source tree (60 kit tests), checks SKILL.md's
  frontmatter (which `validate` never reads), all under a temp `CLAUDE_CONFIG_DIR`, each child in
  its own process group killed on timeout; otherwise one skip line.

## Verification and review

Each task ran implement, then two read-only verifiers (conformance, adversarial), then repair until
clean (Workflow, ultracode). Verify paid on T2 (two repair rounds: control characters in htui's
text, the command-mode timer stop, the skew line cleared after a re-emit) and T4.

`rust-reviewer`: approve with fixes, no CRITICAL/HIGH. **F1 (MEDIUM)**: `--force` accepted any
directory with the right two top-level names, so `--dir crates/htui/claude-plugin --force` would
have deleted uncommitted plugin source; fixed by the every-file ownership check. LOW F2-F7
(trailing-slash symlink, non-UTF-8 path, probe before refusal, untested rollback, gate child
orphans, mod `poll_seconds` bound and stand-down `/htui`) fixed, with the verifiers' lows, in
review-fix lanes R1a (mod), R1b (emit, gate) and R1c (the recheck after the probe, docs). Lows
left by choice: the remaining check-to-rename race on `--force` (no lock; the blueprint accepts it
for the user's own directory), and cosmetic wording.

## Commits

PRD/plan `39a72e1f`, blueprint `910870d3`, amendments `670449f4`. T1 `f4663eb1` `98f2949f`
`c93e1547` `a1c4718a`; T2 `c091607e` `34cded5a` `fde1c1b7` `9719d957` `e8566e68` `689ef462`;
T3 `cc0c140f` `a6785261` `4fcad2f4` `e7bf774c` `50ae120f`; T4 `e4639372` `e32afa4d` `3ae2368c`
`314c4ee2`; review R1a `6b94c200` `157efd33` `80860919` `87d52bf9` `77baef8b` `1f0c7ca0`
`daa11c48` `04f03b28`, R1b `1992f0ae` `3aff3678` `7627a2cb`, R1c `ec611ee1` `6d216916` `be035ff1`,
`3a03d826`.

## Gate

Final workspace gate on `3a03d826` (sandbox Postgres and Qdrant, serial, `--all-features
--no-fail-fast -- --test-threads=1`): **6028 passed, 1 failed, 36 ignored**. The one failure is
`htui-worker` `auto_queue.rs:2210` `a_batch_overshoots_its_cap_by_at_most_one_attempt_pg`, MOD-95,
which fails identically on `main`; MOD-93 touches nothing in `htui-worker`. `cargo fmt --all
--check` clean; `cargo clippy --workspace --all-targets --all-features -- -D warnings` and the
featureless clippy clean. `cargo test -p htui --lib claude_emit` 36 pass, `--bin htui` 7, `--test
claude_plugin` 3 (it ran `claude` 2.1.296: `validate --strict` on both variants, `plugin test` 60
kit tests). A manual `emit` + `claude plugin marketplace add` + `install` under a scratch
`CLAUDE_CONFIG_DIR` gave `htui@htui`, enabled, read from the emitted folder. Workflow-docs
validator: 0 errors, 0 warnings.

## Owed on the host

- The real Postgres gate on the merged tree.
- A live session: `htui claude emit`, `claude plugin marketplace add` + `install`, the band and
  `/htui` against the host's store; `tsc -p` on the mod once a TypeScript toolchain is present (the
  sandbox has none; `validate --strict` and the kit tests stand in).
- Windows (MOD-16's ANA-29 note): `emit` with `htui.exe` reached through `$.process.run`.
- Follow-up: MOD-94's permission buttons land in this pane.

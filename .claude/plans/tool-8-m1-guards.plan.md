# Plan: TOOL-8 M1 — `htui-dev` mod guards

**Source**: `HANDOFF.md` TOOL-8 (from ANA-29, `docs/ANA-29.md` §4.2 D1-D4/D8, §6.1-6.3)
**Routed**: plan path (C3 only fired); maintainer scoped the run to **M1**. M2 (status line, `/gate`,
`/next`, `/validate`) stays open, blocked on TOOL-9.
**Status**: M1 done 2026-10-14 (`1d3963c3`..`40446f29`; review R1 block → fixed, R2 approve with changes → fixed). M2 done 2026-10-14 (`.claude/plans/tool-8-m2-status-commands.plan.md`).
**Complexity**: Medium (one new plugin directory, ~10 files, no Rust change)

## Summary

A skills-directory plugin at `.claude/skills/htui-dev/` whose mod refuses, with a corrective reason,
five kinds of call that cost this repo real losses: vacuous `cargo test` runs (D1), the forbidden
literal written into `crates/` (D2), writes and commits from agents spawned read-only (D3), Fable
subagents (D4), and `git commit --no-verify` (D8). Every guard is a `{ deny }`, never a rewrite, and
fails open with a `$.ui.status` warning. The logic is pure TypeScript (command and input parsing),
so there are no `scripts/` twins in M1.

## Requirements (restated)

| # | Guard | Event | Denies when |
|---|---|---|---|
| G1 | vacuous cargo test | `tool.call` Bash | a simple command `cargo test …` selects a package whose integration tests are feature-gated, integration tests are in scope, and the gating feature stays off once every way cargo turns features on is counted: the command line, the selected crates' `[dependencies]`/`[dev-dependencies]` features, and `[features]` forwarding (table below; R1, claim 23). (R2) A `cd`/`pushd` into `crates/<member>` earlier in the same Bash line makes a `cargo test` with no `-p`/`--workspace`/`--manifest-path` select that member |
| G2 | wrong feature name | `tool.call` Bash | `cargo test -p <crate> --features testkit` where `<crate>` calls it `test-support`, or the reverse (spelling fix only: cargo already errors, probe 10) |
| G3 | forbidden literal | `tool.call` Edit, Write, Gortex and (R1) Serena write tools (R2: also its optional `jet_brains_*` ones), either under an optional `plugin_<name>_` prefix | the *new* text (`new_string`, `content`, `new_source`, Serena's `body`/`repl`, …) contains the literal and the target path is `crates/**/*.rs` or `crates/**/*.json` (any `target/` dir excluded), except `crates/htui-agent/tests/extensibility.rs`. Old text is never checked: removing the literal is the fix |
| G4 | read-only agents | `agent.spawn` (tag) + `tool.call` (deny) | the call's `agentId` was tagged read-only, and the call is Edit, Write, NotebookEdit, a Gortex or Serena write tool, or (R1 widened, R2 exact) Bash running a git write: `commit`, `reset`, `restore`, `merge`, `rebase`, `cherry-pick`, `revert`, `am`, `pull`, `push`, `checkout-index`, `read-tree`, `update-ref`, `update-index`, `gc`, `filter-branch` in any form; `apply` but with only `--check`/`--stat`/`--numstat`/`--summary`; `add`/`mv`/`rm` but `-n`/`--dry-run`; `stash` but `list`/`show`; `checkout` with a pathspec (`.`, after `--`, a second positional, `--pathspec-from-file`, `--ours`/`--theirs`/`-2`/`-3`) or `--force`/`--patch`; `switch` with `--force`/`--discard-changes`/`-C`/`--orphan`; `clean` but `-n`/`--dry-run`; `branch`/`tag` but their list forms; `bisect` but `log`/`view`/`visualize`; `worktree add`/`remove`/`move`/`prune`; `submodule update`/`deinit`/`add`/`sync`; `sparse-checkout` but `list`; `reflog expire`/`delete`; `notes add`/`remove`/`edit`/`append`/`copy`/`prune`; `replace` but its list forms. Long options by unique prefix. `status`, `diff`, `log`, `show`, `blame`, `fetch`, `checkout <branch>`, `switch <branch>` stay allowed; any other git command is a Known gap |
| G5 | Fable spawn | `agent.spawn` | `e.model` names a Fable model (`/fable/i`) |
| G6 | `--no-verify` | `tool.call` Bash | `git … commit` with `--no-verify` or `-n` (also inside a short-flag cluster such as `-an`), unless a later `--verify` cancels it |

**G1 table** (from the Cargo manifests and gated test files, verified claims 8-9, 12, 23; R1
replaced the original table, which claim 11 had made wrong):

| Selection | Refused unless |
|---|---|
| includes `htui` (`-p htui`, `--manifest-path crates/htui/Cargo.toml`, `--workspace` / `--all` / no package selection: virtual root = every member) | `htui` gets `testkit` (`--all-features`, `testkit`, `htui/testkit`); its dev-dependencies turn on agent/core/store `test-support` |
| `-p htui-agent` / `htui-core` (Q1) with none of htui, orch, worker, mcp selected (store's dev-dependencies cover core only) | that crate gets `test-support` (`--all-features`, `test-support`, `<crate>/test-support`, or a forward such as `htui-agent/test-support` → core) |
| `-p htui-store` (Q1) | never: store's own dev-dependency on itself turns `test-support` (and `demo`) on |

Integration tests are out of scope, and G1 allows the run, when the only target selectors are `--lib`,
`--bin(s)`, `--doc` or `--example(s)`. `--tests`, `--test <name>`, `--all-targets`, or no selector at
all count as in scope. Feature spellings accepted: `--features a,b`, `--features "a b"`,
`--features=…`, `-F …`, repeated.

**Shell parsing** (`hooks/shell.ts`, shared by G1/G2/G4/G6): split a Bash command into simple
commands on unquoted `;`, `&&`, `||`, `|`, `&`, newlines, `(`, `)`. Tokenize with single/double
quotes and backslash escapes. Strip leading `VAR=value` assignments and the wrappers `env`,
`command`, `time`, `nice`, `timeout <n>`, `nohup`. Recurse into `bash -c` / `sh -c` string
arguments. A command the parser cannot follow, such as an unbalanced quote, is **allowed**: the
guards are a safety net, not a sandbox (ANA-29 §6.3, "fail open").

**G4 tagging**: in `agent.spawn`, a spawn is read-only when (Q2) its prompt contains the
case-sensitive token `READ-ONLY`, or matches `/strictly no edits/i`, or its `subagentType` is
`rust-reviewer` or `code-architect`, or its `parentAgentId` is already tagged. The hook awaits
`next(e)` and, when the result carries an `agentId`, adds it to a `$.state` atom (`readOnly`, which
survives a module reload, unlike a module-level `Set`). Explore/Plan-type agents need no tag: they
have no write tools.

**Gortex write tools** (Q3) are matched by name: `^mcp__gortex__(edit|edit_file|edit_symbol|batch_edit|rename_symbol|refactor)$`.
G3 reads their new-text fields at any depth (`new_string`, `new_source`, `content`, `new_name`) and
their path from `path`/`file_path`, or from `id` before `::` (`htui/crates/x.rs::Sym`). A
`batch_edit` checks each edit.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Plugin layout | `crates/htui/claude-plugin/htui/.claude-plugin/plugin.json` | `name`, `version`, `description`, `author` (validate `--strict` fails without `author`, probe 5), `types` |
| Mod module | `crates/htui/claude-plugin/htui/hooks/register.tsx` | top-of-file comment saying what the mod does and does not do; small named helpers with `/** */` docs; `register: Register` at the bottom; never throws out of a helper |
| State | `crates/htui/claude-plugin/htui/types/index.d.ts` | `declare module 'claude-code' { interface PluginState { … } }` plus `atom({ plugin, key }, initial)` |
| Guard shape | bundled `plugin-authoring/examples/tool-call.ts` | `on('tool.call', { tool }, …)` returning `{ deny }` or `next(e)`, with `.catch(($, e, next) => next.called ? next(e) : …)` |
| Tests | `crates/htui/claude-plugin/htui/tests/commands.test.ts` | `test('<behaviour sentence>', async ($, on) => …)` from `claude-code/testing`; a bottom hook answers `{ result }` for an allowed call; one behaviour per test, table loops for spellings |
| Errors | ANA-29 §6.3 | deny text = `htui-dev: <what> — <the command or fix to use instead>`; fail open + `$.ui.status('htui-dev: <guard> guard failed, call allowed')` |

## Files to Change

| File | Action | Why |
|---|---|---|
| `.claude/skills/htui-dev/.claude-plugin/plugin.json` | CREATE | the plugin manifest (`htui-dev`, author, `types`) |
| `.claude/skills/htui-dev/hooks/hooks.json` | CREATE | `{"modules": ["./register.ts"]}` |
| `.claude/skills/htui-dev/hooks/register.ts` | CREATE | wires G1-G6 onto `tool.call`/`agent.spawn`, fail-open `.catch` per hook |
| `.claude/skills/htui-dev/hooks/shell.ts` | CREATE | quote-aware splitter/tokenizer, wrapper stripping, `-c` recursion |
| `.claude/skills/htui-dev/hooks/guards.ts` | CREATE | pure decision functions `cargoTest`, `noVerify`, `gitWrite`, `forbiddenLiteral`, `fableModel`, `declaresReadOnly`, each answering a deny reason or `null` |
| `.claude/skills/htui-dev/types/index.d.ts` | CREATE | `PluginState['htui-dev'].readOnly: readonly string[]` |
| `.claude/skills/htui-dev/tests/{shell,cargo,git,edits,agents}.test.ts` | CREATE | kit tests, written first |
| `docs/htui-dev.md` | CREATE | what each guard refuses and why, how to develop (`--plugin-dir`, `claude plugin test`), off switches (`--safe-mode`, `disableAllHooks`) |
| `README.md` | UPDATE | one line under the contributor section pointing to `docs/htui-dev.md` |

No Rust, no `.sqlx`, no snapshots. Nothing under `crates/`, so the extensibility scan never sees the
mod's own fixtures; the tests still spell the literal as `'ze' + 'ta'` so a copy into `crates/`
cannot trip it.

## Tasks

All tasks share `register.ts` and run **serially** in one implementer. No fan-out: the file sets
intersect at `register.ts`/`guards.ts` (step 3.5 rule).

### Task 0: Live probe (D3 and the trust question)
- **Action**: under a scratch `CLAUDE_CONFIG_DIR` (claudeAiOauth minus refreshToken, onboarding and
  trust flags seeded, parent `CLAUDE*`/`ANTHROPIC*` stripped; memory
  `claude-live-probes-use-scratch-config`), copy the repo to a scratch git checkout holding a throwaway
  skills-directory probe mod that logs `agent.spawn` (`workflow`, `agentId`) and the `agentId` of each
  `tool.call` to a `/tmp` file. Run `claude -p --model haiku --debug-file …` twice: once with a
  plain Agent call, once asking for a one-agent Workflow script. Record (a) whether the module loads
  with no prompt beyond trust ("hooks module … loaded"), (b) whether a Workflow agent raises
  `agent.spawn` and its tool calls carry the answered `agentId`, (c) whether the Workflow tool is
  offered in `-p` at all.
- **Fallback**: if Workflow is not available in `-p`, D3 for Workflow agents stays **UNVERIFIED**
  with the types' statement (`claude-code.d.ts`, `AgentSpawnInput.workflow`) as evidence, and the
  doc says so.
- **Validate**: probe notes written into this plan's verified-claims table; nothing installed into
  `~/.claude`; scratch dirs removed.

### Task 1: Skeleton + shell parser (TDD)
- **Action**: `plugin.json`, `hooks.json`, `types/index.d.ts`, an empty `register`; `tests/shell.test.ts`
  first (splitting on each separator, quotes, escapes, assignments, wrappers, `bash -c` recursion,
  unbalanced quote → `null`), then `shell.ts`.
- **Mirror**: MOD-93's plugin manifest and test style.
- **Validate**: `claude plugin test .claude/skills/htui-dev` and `claude plugin validate --strict
  .claude/skills/htui-dev` under a scratch `CLAUDE_CONFIG_DIR`.

### Task 2: G1 + G2 cargo guards (TDD)
- **Action**: `tests/cargo.test.ts` first, table-driven over the G1 table (deny and allow rows, every
  feature spelling, `--lib`/`--doc` allow, `--test x` deny, `cd crates/htui && cargo test` treated
  as no selection (R2: as selecting `htui`), `cargo build`/`cargo clippy` untouched), then `guards.ts::cargoTest` and its Bash
  hook.
- **Validate**: `claude plugin test`.

### Task 3: G6 `--no-verify` + G4 git half (TDD)
- **Action**: `tests/git.test.ts` first (`-n`, `-an`, `-nm msg` is a deny but `-mn` is a message,
  `--verify` after cancels, `git -C dir commit --no-verify`, `git log --no-verify`-like non-commit
  allowed, `git commit` inside `bash -c`), then `noVerify` and `gitWrite`.
- **Validate**: `claude plugin test`.

### Task 4: G3 forbidden literal (TDD)
- **Action**: `tests/edits.test.ts` first (Edit/Write/each Gortex write tool; `.rs` and `.json` under
  `crates/`, an absolute path, a Windows path with `\`, a worktree path
  `.claude/worktrees/x/crates/…`, `target/` excluded, the extensibility test excepted, old text
  ignored, non-crates path allowed, `.ts`/`.md` under crates allowed), then `forbiddenLiteral`.
- **Validate**: `claude plugin test`.

### Task 5: G4 tagging + G5 Fable (TDD)
- **Action**: `tests/agents.test.ts` first (each read-only marker; a lowercase "read-only host
  tree" not tagged; `rust-reviewer` tagged by type; a nested spawn under a tagged parent tagged; a
  tagged agent's Edit/Write/Gortex/`git commit` denied and its Read/`cargo check` allowed; an
  untagged agent and the main loop allowed; the tag surviving a module reload; Fable by alias and by
  full id denied, `opus`/undefined allowed), then the hooks.
- **Validate**: `claude plugin test`.

### Task 6: Fail-open + reach
- **Action**: a test per hook where the decision throws (seeded through a malformed input), asserting
  the call goes through and `$.ui.status` was set. Check validate's `calls:` line is only `$.state.*`
  and `$.ui.status` (no `$.process`, `$.fs`, `$.http`, `$.model`, ANA-29 §6.3).
- **Validate**: `claude plugin validate --strict` output read by hand and quoted in the write-up.

### Task 7: Docs
- **Action**: `docs/htui-dev.md` and the README line. Mention that `disableAllHooks`/`--safe-mode`
  turn it off, that MOD-91 keeps it out of htui-driven sessions, and that M2 is to come.
- **Validate**: workflow validator (close-out).

## Validation

```bash
# scratch config, never the shared ~/.claude
export CLAUDE_CONFIG_DIR=$(mktemp -d)
env -u CLAUDECODE claude plugin validate --strict .claude/skills/htui-dev
env -u CLAUDECODE claude plugin test .claude/skills/htui-dev
# no Rust changed, but the literal scan is the workspace's own check:
cargo test -p htui-agent --all-features --test extensibility
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

Not gated by cargo until TOOL-9 wires `validate`/`test` for `.claude/skills/htui-dev` into
`scripts/gate.sh` (it is in TOOL-9's text already).

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| A guard blocks a legitimate command (parser over-matches) | Medium | narrow per-simple-command matching, allow rows in every test table, deny text names the fix; `--safe-mode` as an escape |
| A subagent's first tool call races the tag (`agentId` known only after `next`) | Low | the tag is written right after `next` settles, before the subagent's first model turn returns; documented |
| Workflow agents not raised in practice | Low-medium | Task 0 probe; fallback documented as UNVERIFIED |
| G1 table drifts from the manifests | Medium | table lives in one constant with a comment naming the manifests; TOOL-9's gate is the place for a drift check |
| Mods API changes | High (per release) | kit tests + strict validate; TOOL-9 gate |
| The skills-dir plugin prompts beyond trust in some sessions | Low | Task 0 (a) |

## Decisions for the maintainer (asked at CONFIRM)

**CONFIRMED 2026-10-14: Q1, Q2 and Q3 all yes, as recommended.**

- **Q1** Also deny `-p htui-agent|htui-core|htui-store` without `test-support`? They gate 6/8/3
  integration-test files the same way. *Recommended: yes* (same vacuous-green class).
- **Q2** Read-only marker = case-sensitive `READ-ONLY`, or `strictly no edits`, or type
  `rust-reviewer`/`code-architect`, or a tagged parent. *Recommended: yes*. Plain lowercase
  "read-only" appears in unrelated prompt text ("the read-only host tree").
- **Q3** Cover Gortex MCP write tools in G3/G4, not only Edit/Write. *Recommended: yes*: this repo's
  instructions route most edits through them, and `MultiEdit` no longer exists (claim 1).

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| 1. `MultiEdit` is a tool to guard (item text) | **Falsified** → amended to Edit/Write/NotebookEdit + Gortex write tools | 2.1.296 `claude-code.d.ts` `BuiltinToolInputs`: Edit, NotebookEdit, Write; no MultiEdit |
| 2. `agent.spawn` fires for Workflow-tool agents | Verified on paper; live check in Task 0 | `claude-code.d.ts` `AgentSpawnInput`: "also raised for each agent a workflow script's `agent()` starts"; `workflow` field: "A hook can only refuse it" |
| 3. A subagent's `tool.call` carries its `agentId` | Verified | `claude-code.d.ts` `AgentLoop.agentId`; `AgentSpawnResult.agentId`: "the same string its loop's `tool.call` events carry" |
| 4. `{ deny }` from `agent.spawn` must come before `next` | Verified | `AgentSpawnResult` doc: a deny after `next` "fails the hook" |
| 5. `validate --strict` passes a mod with gating hooks | Verified (scratch probe) | lists "gating hook with/without .catch", passes once `author` is set; fails on missing `author` |
| 6. The test kit raises `tool.call`/`agent.spawn` and returns a hook's deny | Verified (scratch probe) | `claude plugin test` 1 pass with a `{ result }` bottom for Bash |
| 7. The literal scan covers `crates/**/*.{rs,json}` minus `target/` and its own file, case-sensitive | Verified | `crates/htui-agent/tests/extensibility.rs` `tree_files`, `the_codebase_has_never_heard_of_zeta` (the doc comment says src/tests/seeds; the code recurses all of `crates/`) |
| 8. Only `htui` calls it `testkit`; agent/core/orch/store/worker `test-support`; mcp/secrets neither | Verified | `crates/*/Cargo.toml` `[features]` |
| 9. Feature-gated test files: htui 41 `testkit`; agent 6, core 8, store 3 `test-support` | **Corrected in R1**: htui 43 `testkit`; agent 4, core 8 `test-support`; store 3 `test-support` + 5 `demo` | `#![cfg(…)]` lines over `crates/*/tests/*.rs` and `*/main.rs` (htui: 39 plain, 2 beside a separate `#![cfg(unix)]`, 2 as `all(…)`) |
| 10. `-p <test-support crate> --features testkit` is vacuous | **Falsified**: cargo errors "does not contain this feature" | scratch workspace probe (cargo, resolver 3) |
| 11. `--workspace --features testkit` covers the workspace | **Falsified in this repo: dev-dependency enables** (R1). The original "falsified" came from a scratch workspace without them; here the run is real | `cargo tree --workspace -e features,normal,dev` shows agent/core/store `test-support` on with no flags; see claim 23 |
| 12. Root manifest is virtual; bare `cargo test` = every member | Verified | `Cargo.toml` has `[workspace]` only |
| 13. Verifier prompts mark read-only as `READ-ONLY` / "STRICTLY no edits, no commits" | Verified | memory `ultracode-scoped-to-implementers` (MOD-12 M3) |
| 14. `git commit -n` = `--no-verify`; `--verify` is its opposite | Verified | `git commit -h` |
| 15. Live probe auth available in this sandbox | Verified | `~/.claude/.credentials.json` `expiresAt` ≈ 7.5 h ahead |
| 17. Task 0 (a): a skills-directory plugin loads with no prompt beyond workspace trust | Verified live (2.1.296, `-p`, scratch config) | debug log: "hooks module probe@skills-dir loaded (worker, environment 1, tier user)", "plugin.register: probe … judged by core alone: admitted" |
| 18. Task 0 (b): a plain subagent's `agentId` is answered by `next` before its first `tool.call` | Verified live | spawn settled 08:58:36.478 with `agentId` af39…; that agent's Bash `tool.call` at 08:58:46 carried it. The parent's `Agent` tool call itself raises `tool.call` on main |
| 19. Task 0 (b): Workflow-tool agents raise `agent.spawn`, and their tool calls carry the answered `agentId` | **Verified live** (D3 closed) | `subagentType` `workflow-subagent`, `workflow: {runId: "wf_…", agentIndex: 1}`, `parentAgentId` absent; the agent's Bash `tool.call` carried the same id |
| 20. Task 0 (c): the Workflow tool is available under `-p`; a workflow agent's prompt and `model` option reach the hook | Verified live | prompt seen verbatim (`"READ-ONLY verifier. …"`), `model=sonnet` → answered `claude-sonnet-5-5` |
| 21. Live smoke (post-implementation): G6, G1, G3 refuse in a real `-p` session with their corrective texts | Verified live | scratch checkout, haiku: `--no-verify` commit refused (no commit made), `cargo test -p htui` refused, Write of the literal under `crates/` refused (file unchanged) |
| 22. Live smoke: a `READ-ONLY` subagent's Write is refused, its `git status` runs | Verified live | "agent a6e0… was started read-only and may not change files (Write)"; `notes.txt` absent |
| 16. Tasks independent | No: all share `register.ts`/`guards.ts` | serial, one implementer |
| 23. (R1) cargo turns test features on through the selected crates' dependencies and forwarding, unified over the selection | Verified | `[dev-dependencies]`: htui → agent, orch, worker, store `test-support`; orch → core, agent; worker and mcp → core, agent, orch, store; store → core and itself. `[features]`: `htui/testkit` → store; agent → core; orch → core, agent; core → `demo`; store → `demo`. `cargo tree -p <sel> -e features,normal,dev`: `-p htui-store` has store `test-support`; `-p htui-agent` has none; `-p htui-agent -p htui-orch` has agent `test-support` |

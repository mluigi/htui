# Guards for developing htui (`.claude/skills/htui-dev`)

`htui-dev` is a Claude Code mod that lives in this repository and loads in every Claude Code session
you open here. It refuses six kinds of tool call that have cost this repo real work: test runs that
pass without testing anything, a name the extensibility test forbids written into `crates/`, writes
and commits from agents that were started read-only, Fable subagents, and `git commit --no-verify`.
Each refusal says what to run instead.

It is a safety net, not a sandbox. It never rewrites a call, it lets through anything it cannot
read, and you can turn it off for a session.

The design is in [`.claude/plans/tool-8-m1-guards.plan.md`](../.claude/plans/tool-8-m1-guards.plan.md)
(TOOL-8 M1). The reasons behind each guard are in [`docs/ANA-29.md`](ANA-29.md), §4.2 (D1-D4, D8).
This is milestone 1. Milestone 2 (a status line and the `/gate`, `/next` and `/validate` commands)
is still to come, after TOOL-9.

- [What it refuses](#what-it-refuses)
- [How each guard decides](#how-each-guard-decides)
- [What it never does](#what-it-never-does)
- [Turning it off](#turning-it-off)
- [Working on the mod](#working-on-the-mod)
- [Known gaps](#known-gaps)
- [What the live probe showed](#what-the-live-probe-showed)

## What it refuses

Every refusal starts with `htui-dev:`, names the problem, and ends with the fix after a `—`.

| Guard | Refuses | Why | The refusal's fix |
|---|---|---|---|
| G1 | `cargo test` that builds feature-gated integration tests without their feature | The gated files compile to nothing, so the run reports `ok` with 0 tests (ANA-29 D1) | Your command with `--features testkit` (or `test-support`) added, or `--all-features` for several crates or the whole workspace |
| G2 | `cargo test -p <crate>` naming the other crate's test feature | `htui` calls it `testkit`, the other crates `test-support`. cargo refuses the wrong one anyway; the refusal just gives you the right spelling | Your command with the name swapped |
| G3 | An edit that writes the forbidden literal into `crates/**/*.{rs,json}` | `crates/htui-agent/tests/extensibility.rs` fails the workspace gate on it, but per-crate gates stay green, so it is caught lanes later (ANA-29 D2) | Use another name. Only `extensibility.rs` may spell it |
| G4 | File writes, and `git commit`, `git stash`, `git reset` or `git checkout --`, from an agent started read-only | A read-only verifier once fixed and committed the defect it was asked to report (ANA-29 D3) | Report the change in the answer instead |
| G5 | A subagent spawned with a Fable model | Every agent here inherits the session's model and none runs on Fable (ANA-29 D4) | Leave `model` unset |
| G6 | `git commit --no-verify` or `git commit -n` | It skips the repo's commit hooks (ANA-29 D8) | The same commit without the flag |

## How each guard decides

### Reading a Bash command (G1, G2, G4, G6)

The Bash guards look at each simple command in the line, not at the raw text:

- The line is split on unquoted `;`, `&&`, `||`, `|`, `|&`, `&`, newlines, `(`, `)`, `$(` and
  backticks. Redirections such as `2>&1` and `&>log` are dropped, not split on.
- Quotes and backslashes are read as the shell reads them. A heredoc body and a `#` comment are
  skipped, so `git commit -F - <<'EOF'` with a message about `--no-verify` is fine, and so is
  `git commit -m "$(cat <<'EOF' … EOF)"`.
- Leading `VAR=value` assignments, `if`/`then`/`do`/`{` and similar words, and the wrappers `env`,
  `command`, `time`, `nice`, `timeout <duration>` and `nohup` are stripped, with their options.
- `bash -c '…'` and `sh -c '…'` (also `-lc`, `-ec`) are followed into their string.

So `echo cargo test -p htui` and `git commit -m "cargo test -p htui"` are left alone, while
`RUST_LOG=x timeout 900 cargo test -p htui 2>&1 | tail` is checked.

A line the reader cannot follow (an unbalanced quote, a heredoc with no end) is let through
without comment.

### G1 and G2: `cargo test`

Each member's integration tests are gated on one feature, read from the `Cargo.toml` files and the
`#![cfg(feature = …)]` lines under `crates/*/tests`:

| Selection | Needs one of |
|---|---|
| `-p htui` (also `--package`, `--package=`, `-phtui`, `--manifest-path crates/htui/Cargo.toml`) | `--all-features`, `testkit`, `htui/testkit` |
| `-p htui-agent`, `htui-core`, `htui-store` | `--all-features`, `test-support`, `<crate>/test-support` |
| `--workspace`, `--all`, or no package at all (the root is a virtual manifest, so that means every member) | `--all-features`, or every crate's own feature (`testkit,test-support`). `--workspace --features testkit` still runs the `test-support` crates' gated files as 0 tests |
| `-p htui-mcp`, `htui-secrets`, `htui-orch`, `htui-worker` | nothing: they have no gated integration tests |

Features are read in every spelling cargo takes: `--features a,b`, `--features "a b"`,
`--features=…`, `-F …`, `-F…`, repeated. `--exclude` takes a crate out of a workspace run.

The integration tests count as in scope unless the only target selectors are `--lib`, `--bin(s)`,
`--doc`, `--example(s)` or `--bench(es)`. `--tests`, `--test <name>`, `--all-targets`, or no
selector at all, bring them in. A flag after `--` belongs to the test binary, so
`cargo test -p htui -- --lib` is still refused.

`cargo t`, `cargo +nightly test` and cargo's own options before `test` (`-q`, `--locked`,
`--color …`, `--config …`, `-Z …`) are recognised. `cargo build`, `check`, `clippy` and
`nextest` are not checked.

G2 fires when a selected crate is given the other test feature name, plain or qualified:
`-p htui-agent --features testkit`, `-p htui --features test-support`,
`--features htui-store/testkit`.

### G3: the forbidden literal

G3 reads the edit tools: `Edit`, `Write`, `NotebookEdit`, and Gortex's `edit_file`, `write_file`,
`edit_symbol`, `rename_symbol`, `batch_edit`, `edit` and `refactor`. It reads only the text the call
would write (`new_string`, `new_source`, `content`, `new_name`), never the old text, so removing the
literal is always allowed. The check is case-sensitive, like the test.

The target path comes from `file_path`, `path`, `notebook_path`, `target.file`, or a symbol id's
file part (`htui/crates/x.rs::Sym`). In a `batch_edit`, each edit is checked under its own path. The
guard refuses when the path, from its last `crates/` folder on, ends in `.rs` or `.json`, has no
`target/` folder, and is not `crates/htui-agent/tests/extensibility.rs`. That is the same set of
files the test scans. Absolute paths, Windows `\` paths and worktree copies
(`.claude/worktrees/x/crates/…`) are all matched.

### G4: read-only agents

When a subagent starts, it is tagged read-only if:

- its prompt contains `READ-ONLY` (case-sensitive: "the read-only host tree" is ordinary text);
- its prompt contains "strictly no edits", in any case;
- its type is `rust-reviewer` or `code-architect`; or
- the agent that spawned it is tagged.

The tag is the agent's `agentId`. It is kept in the mod's `$.state`, which outlives a reload of the
mod, for the rest of the session. A tagged agent's `Edit`, `Write`, `NotebookEdit` and Gortex write
calls are refused, and so are its Bash `git commit`, `git stash` (but not `stash list` or
`stash show`), `git reset` and `git checkout --`. It can still read, search, build, check and run
tests. Explore and Plan agents need no tag: they have no write tools.

Workflow agents are covered too: a workflow script's `agent()` raises the same spawn event, and its
tool calls carry the same `agentId` (verified live, see below).

### G5: Fable subagents

G5 reads the Agent tool's `model` parameter and refuses any value matching `fable` in any case
(`fable`, `claude-fable-…`). A refusal has to come before the agent starts, and only that parameter
is known by then. See [Known gaps](#known-gaps) for what that misses.

### G6: `--no-verify`

G6 refuses `git commit` with `--no-verify`, an abbreviation git accepts (`--no-ver`), or `-n`, also
inside a cluster of short flags (`-an`, `-nm msg`). A later `--verify` cancels it. A value is not a
flag: `-m -n` and `-mn` are messages, `-Cn` names a commit. Git's own options before `commit`
(`-C dir`, `-c k=v`, `--git-dir`, …) are skipped. Other commands (`git log -n 3`,
`git push --no-verify`) are not checked. The refusal shows the commit with the flag taken out
(`-an -m x` becomes `-a -m x`).

## What it never does

- **It never rewrites a call.** Every guard answers with a refusal or lets the call through
  unchanged. (In auto mode a rewritten call is denied outright, ANA-29 §2.3.)
- **It fails open.** If a guard throws, or the tag store cannot be read or written, the call goes
  through and the status line says which guard failed, for example
  `htui-dev: read-only guard failed, call allowed`. A command the parser cannot read is let through
  silently.
- **It reaches nothing outside the event.** No process, file, network or model call. The only
  things it calls are `$.state` for the read-only tags and `$.ui.status` for those warnings.
  `claude plugin validate` lists exactly `$.state.get, $.state.set, $.ui.status`.

## Turning it off

- `claude --safe-mode` (or `CLAUDE_CODE_SAFE_MODE=1`) starts one session with every
  customization off, this mod included.
- `"disableAllHooks": true` in any settings file stops every mod and settings hook, this one
  included. The plugin's other parts would stay loaded, but this one has none.

There is no switch for a single guard. If a guard refuses a legitimate command, the refusal's fix is
usually what you meant. If it is not, use `--safe-mode` for that session and report the case.

**Sessions htui starts:** since MOD-91, htui's own agent sessions leave out project mods, so
`htui-dev` does not load there unless that agent's row has `mods (y/n)` set to `y`. See
[`docs/htui-mcp.md`](htui-mcp.md#isolation-from-your-claude-code-setup).

## Working on the mod

```
.claude/skills/htui-dev/
  .claude-plugin/plugin.json   name, version, author (validate --strict needs it), types
  hooks/hooks.json             {"modules": ["./register.ts"]}
  hooks/register.ts            the three hooks, the fail-open helpers, the readOnly state
  hooks/guards.ts              the pure decisions, the G1 crate table, the refusal texts
  hooks/shell.ts               the Bash command reader
  types/index.d.ts             the $.state contract (htui-dev.readOnly)
  tests/*.test.ts              the kit tests; tests/fixtures.ts holds their helpers
```

The mod is a skills-directory plugin: a session opened in this repo loads it once you trust the
workspace, with no install step. An interactive session watches the folder and reloads the mod when
you save. To load a copy from elsewhere, use `claude --plugin-dir <folder>`. When the engine loads
it from a folder it watches, it writes its type files into `.claude-plugin/types/`, which is
git-ignored.

Run the tests and the validator under a scratch configuration, never your own `~/.claude`, and
with the parent session's Claude variables removed:

```
C=$(mktemp -d)
env -u CLAUDECODE $(env | cut -d= -f1 | grep -E '^(CLAUDE|ANTHROPIC)' | sed 's/^/-u /') \
  CLAUDE_CONFIG_DIR=$C claude plugin test .claude/skills/htui-dev
env -u CLAUDECODE $(env | cut -d= -f1 | grep -E '^(CLAUDE|ANTHROPIC)' | sed 's/^/-u /') \
  CLAUDE_CONFIG_DIR=$C claude plugin validate --strict .claude/skills/htui-dev
rm -rf "$C"
```

`validate` exits 0 even when it fails: look for `✔ Validation passed`. Its `calls:` line must stay
`$.state.get, $.state.set, $.ui.status`.

The tests use `claude-code/testing`: hooks a test registers sit beneath the mod and stand in for the
engine. The kit passes a call's `agentId` and a spawn's `parentAgentId` through to the mod, so the
read-only tests run through the real hooks. The parser and the git table are also tested directly.

For type-checking, use the `tsconfig.json` from the header of the `claude-code.d.ts` that
Claude Code writes, kept **outside** the mod folder (`.claude-plugin/types/` is regenerated on every
load).

Two rules for anything under `.claude/skills/htui-dev/`:

- Never spell the forbidden literal. Build it as `'ze' + 'ta'`, as `guards.ts` and the test fixtures
  do. Nothing here is under `crates/`, so the scan never reads it, but a copied line could end up
  there.
- Keep the G1 crate table (`GATED` in `guards.ts`) in step with the `[features]` of
  `crates/*/Cargo.toml` and the gated test files. Nothing checks this until TOOL-9 adds the mod's
  tests and validator to `scripts/gate.sh`.

## Known gaps

- **G5 sees only an explicit model.** A subagent with no `model` parameter takes its model from its
  agent definition, then from the parent, and that happens after the hook could refuse it. A
  refusal after the agent starts would fail the hook. So a Fable model pinned in an agent definition,
  or inherited from a Fable session, starts anyway, and the mod only shows a status line:
  `htui-dev: subagent <id> resolved to Fable (<model>) — inherited or pinned in its agent definition`.
- **G1 does not know the working directory.** `cd crates/htui && cargo test --features testkit` is
  read as a workspace run and refused (cargo would test only `htui` there). Use `-p htui` from the
  root. A `cargo test` in another project from a session opened here is read the same way.
- **The reader is not a shell.** It does not follow `eval`, scripts, aliases, functions, or a
  `$( … )` inside double quotes, and it treats every `$VAR` as text. A guard can be walked around
  on purpose. It is there to catch mistakes.
- **G4's tag lands as the spawn returns.** The tag is written as soon as the engine answers the
  spawn with the agent's id, before that agent's first model turn returns. The live probe saw a
  10-second gap before the first tool call.
- **The field names of Gortex's compact `edit` and `refactor` tools were not confirmed.** The
  session that built this exposed only the older names (`edit_file`, `write_file`, `edit_symbol`,
  `rename_symbol`, `batch_edit`), whose fields were checked. The compact tools are matched by name
  and read with the same fields, plus `target.file` and `target.symbol`.

## What the live probe showed

TOOL-8's Task 0 ran a throwaway probe mod against Claude Code 2.1.296 under `claude -p`, with a
scratch configuration:

- A skills-directory plugin loads with no prompt beyond workspace trust (`hooks module
  probe@skills-dir loaded`).
- A plain subagent's `agentId` comes back from the spawn before its first tool call, and that tool
  call carries the same id.
- Workflow-tool agents raise the spawn event (`subagentType` `workflow-subagent`, a `workflow`
  field, no `parentAgentId`), and their tool calls carry the `agentId` the spawn answered. So G4
  covers them.
- The Workflow tool is available under `-p`. A workflow agent's prompt and `model` option reach the
  hook as given (`"READ-ONLY verifier. …"`, `model=sonnet`).

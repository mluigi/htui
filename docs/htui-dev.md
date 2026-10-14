# Guards for developing htui (`.claude/skills/htui-dev`)

`htui-dev` is a Claude Code mod that lives in this repository and loads in every Claude Code session
you open here. It refuses five kinds of tool call that have cost this repo real work: test runs that
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
| G1 | `cargo test` that builds feature-gated integration tests with their feature still off | The gated files compile to nothing, so the run reports `ok` with 0 tests (ANA-29 D1) | Your command with `--features testkit` (or `test-support`) added; for several crates or the whole workspace, `--all-features` works too |
| G2 | `cargo test -p <crate>` naming the other crate's test feature | `htui` calls it `testkit`, the other crates `test-support`. cargo refuses the wrong one anyway; the refusal just gives you the right spelling | Your command with the name swapped |
| G3 | An edit that writes the forbidden literal into `crates/**/*.{rs,json}` | `crates/htui-agent/tests/extensibility.rs` fails the workspace gate on it, but per-crate gates stay green, so it is caught lanes later (ANA-29 D2) | Use another name. Only `extensibility.rs` may spell it |
| G4 | File writes, and the git commands [listed under G4](#g4-read-only-agents) that change the working tree, the index, refs, history or the remote, from an agent started read-only | A read-only verifier once fixed and committed the defect it was asked to report (ANA-29 D3). On a shared tree, `git checkout .` or `git clean` would also destroy the parent's uncommitted work | Report the change in the answer instead |
| G5 | A subagent spawned with a Fable model | Every agent here inherits the session's model and none runs on Fable (ANA-29 D4) | Leave `model` unset |
| G6 | `git commit --no-verify` or `git commit -n` | It skips the repo's commit hooks (ANA-29 D8) | The same commit without the flag |

## How each guard decides

### Reading a Bash command (G1, G2, G4, G6)

The Bash guards look at each simple command in the line, not at the raw text:

- The line is split on unquoted `;`, `&&`, `||`, `|`, `|&`, `&`, newlines, `(`, `)`, `$(` and
  backticks. Redirections such as `2>&1` and `&>log` are dropped, not split on.
- Quotes and backslashes are read as the shell reads them. A heredoc body and a `#` comment are
  skipped, so `git commit -F - <<'EOF'` with a message about `--no-verify` is fine.
- A `$( … )` inside double quotes is read as the shell reads it: a fresh quoting context, up to its
  balanced `)`, stepping over nested quotes, parentheses and heredoc bodies. So the usual
  `git commit -m "$(cat <<'EOF' … EOF)"` is one word, even when the message has `"` or `&&` in it,
  and the commands inside the substitution are checked too. `$(( … ))` is arithmetic, not a command,
  quoted or not, and a here-string (`<<<`) is no heredoc.
- Leading `VAR=value` assignments, `if`/`then`/`do`/`{` and similar words, and the wrappers `env`,
  `command`, `time`, `nice`, `timeout <duration>`, `nohup` and `exec` are stripped, with their
  options. `env -S "…"` (`--split-string`) is split into the command it runs.
- `bash -c '…'`, `sh -c '…'`, `zsh -c`, `dash -c` and `ksh -c` (also `-lc`, `-ec`) are followed into
  their string. `--rcfile f`, `--init-file f`, `-o opt` and `-O opt` take their value.
- An array assignment, `arr=( … )` or `arr+=( … )`, is one word, not a subshell.

So `echo cargo test -p htui` and `git commit -m "cargo test -p htui"` are left alone, while
`RUST_LOG=x timeout 900 cargo test -p htui 2>&1 | tail` is checked.

A line the reader cannot follow (an unbalanced quote, a heredoc with no end, nesting so deep that
reading it would overflow the stack) is let through without comment. The one exception is a read-only agent's line: it still meets a coarse pattern for
the git commands that always write (see G4).

### G1 and G2: `cargo test`

**Which tests are gated.** The `#![cfg(feature = …)]` lines under `crates/*/tests`:

| Crate | Gated integration-test files |
|---|---|
| `htui` | 43 on `testkit` |
| `htui-agent` | 4 on `test-support` |
| `htui-core` | 8 on `test-support` |
| `htui-store` | 3 on `test-support`, 5 on `demo` |

The other crates have no gated integration tests.

**What turns a feature on.** cargo turns test features on in three ways, and G1 models all three:

1. **The command line:** `--all-features`, a plain name (applied to every selected crate that has
   that feature), or `<crate>/<feature>`.
2. **The selected crates' own dependencies.** A test build uses each selected crate's
   `[dependencies]` and `[dev-dependencies]`, and cargo unifies their features over every selected
   crate:

   | Selecting | Turns on `test-support` in |
   |---|---|
   | `htui` | agent, orch, worker, store (core through forwarding) |
   | `htui-orch` | core, agent |
   | `htui-worker` | core, agent, orch, store |
   | `htui-mcp` | core, agent, orch, store |
   | `htui-store` | core, and store itself (it lists itself as a dev-dependency) |
   | `htui-agent`, `htui-core`, `htui-secrets` | nothing |

3. **Forwarding** in the `[features]` tables:
   - `htui/testkit` → `htui-store/test-support`
   - `htui-agent/test-support` → `htui-core/test-support`
   - `htui-orch/test-support` → `htui-core/test-support` and `htui-agent/test-support`
   - `htui-core/test-support` → `htui-core/demo`
   - `htui-store/test-support` → `htui-store/demo`

A run is refused only when a selected crate with gated tests would still have its gate off. In
practice:

| Selection | Refused unless |
|---|---|
| includes `htui` (also `--workspace`, `--all`, or no package at all: the root is a virtual manifest, so that means every member) | `htui` gets `testkit`: `--features testkit`, `htui/testkit` or `--all-features`. Nothing else is needed; `htui`'s dev-dependencies cover the rest |
| `-p htui-agent` or `-p htui-core`, with none of htui, orch, worker or mcp beside it (store covers core, not agent) | that crate gets `test-support` |
| `-p htui-store` | never: its own dev-dependency turns `test-support` (and so `demo`) on |
| `-p htui-mcp`, `htui-secrets`, `htui-orch`, `htui-worker` | never: they have no gated integration tests |

So `cargo test -p htui-agent -p htui-orch` and `cargo test --workspace --features testkit` are allowed,
`cargo test -p htui-agent` alone is refused, and a bare `cargo test` is refused for `htui` alone,
with `--features testkit` (or `--all-features`) as the fix.

**How the selection is read.** `-p`, `--package`, `--package=`, `-phtui` and short clusters such as
`-qp htui`; a package's `@version` is dropped and a glob (`-p 'htui-*'`) is matched against the
members. `--manifest-path` names a member only when its folder sits in `crates/` (or `../`) or the
path is a bare `<member>/Cargo.toml`; any other manifest is the workspace root, whatever its folder is
called (`.claude/worktrees/tool-8/Cargo.toml`). `--exclude` (globs too) takes a crate out of a
workspace run.

**The working directory.** A `cd` (or `pushd`) earlier in the same Bash line into a member's folder,
`crates/<member>` or any folder below it, makes a later `cargo test` with no `-p`, `--workspace` or
`--manifest-path` select that member, as cargo would. So `cd crates/htui-store && cargo test` is
allowed, and `cd crates/htui-agent && cargo test` is refused with `--features test-support` as the
fix. Relative paths are joined onto the earlier `cd`s, with `.` and `..` resolved. A `cd` anywhere
else, `cd` alone, `cd -`, `~`, a `$VAR`, `pushd +N` and `popd` select no member again. Where the
line starts is not known; see [Known gaps](#known-gaps).

Features are read in every spelling cargo takes: `--features a,b`, `--features "a b"`,
`--features=…`, `-F …`, `-F…`, inside a short cluster (`-qF testkit`), repeated.

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

G3 reads the edit tools:

- `Edit`, `Write` and `NotebookEdit`;
- Gortex's `edit_file`, `write_file`, `edit_symbol`, `rename_symbol`, `batch_edit`, `edit` and
  `refactor`;
- Serena's `create_text_file`, `replace_content`, `replace_in_files`, `replace_symbol_body`,
  `insert_after_symbol`, `insert_before_symbol`, `rename_symbol`, `safe_delete_symbol`,
  `replace_lines`, `insert_at_line` and `delete_lines`, and its optional JetBrains tools
  `jet_brains_rename`, `jet_brains_move`, `jet_brains_safe_delete` and `jet_brains_inline_symbol`
  (also matched as `jetbrains_…`). Serena is configured machine-wide, so its tools reach sessions
  here.

Either server's tools also count under a plugin's prefix (`mcp__plugin_<name>_gortex__…`).

It reads only the text the call would write (`new_string`, `new_source`, `content`, `new_name`,
and Serena's `body` and `repl`), never the old text, so removing the literal is always allowed. The
check is case-sensitive, like the test.

The target path comes from `file_path`, `path`, `notebook_path`, Serena's `relative_path`,
`target.file`, or a symbol id's file part (`htui/crates/x.rs::Sym`). In a `batch_edit`, each edit is
checked under its own path. The path is read by its segments, with `.` dropped and `..` resolved.
The guard refuses when, from the first `crates` folder on, the path ends in `.rs` or `.json`, has no
`target` folder, and is not `crates/htui-agent/tests/extensibility.rs`. That is the same set of files
the test scans. Absolute paths, Windows `\` paths and worktree copies
(`.claude/worktrees/x/crates/…`) are all matched.

Serena's `replace_in_files` reaches many files. It counts when its `relative_path` is empty or `.`
(the whole project), a folder in `crates/` outside `target/`, or a scanned file. A dry run
(`dry_run: true`) writes nothing and passes G3 and G4.

The Serena tool names and fields were checked against Serena 2.0.0's source. Its JetBrains tools
are named `jet_brains_*` there (class `JetBrainsRenameTool` becomes `jet_brains_rename`);
`jet_brains_rename` writes its `new_name`, and the other three write no new text, so they meet G4
only. Two Serena tools are not matched:

- `execute_shell_command` runs a shell command. Serena's `claude-code` context excludes it, so it
  is not offered here; see [Known gaps](#known-gaps).
- The memory tools (`write_memory`, `edit_memory`, …) write only Serena's memory folder, never the
  tree.

### G4: read-only agents

When a subagent starts, it is tagged read-only if:

- its prompt contains `READ-ONLY` (case-sensitive: "the read-only host tree" is ordinary text);
- its prompt contains "strictly no edits", in any case;
- its type is `rust-reviewer` or `code-architect`;
- the agent that spawned it is tagged; or
- it is a workflow agent whose run was started by a tagged agent's `Workflow` call (see below).

The tag is the agent's `agentId`. It is kept in the mod's `$.state`, which outlives a reload of the
mod, for the rest of the session. Explore and Plan agents need no tag: they have no write tools.

A tagged agent's file writes are refused: `Edit`, `Write`, `NotebookEdit`, and every Gortex and
Serena write tool G3 reads. So are exactly these Bash git commands, which change the working tree,
the index, refs, history or the remote:

- `commit`, `reset`, `restore`, `merge`, `rebase`, `cherry-pick`, `revert`, `am`, `pull`, `push`,
  `checkout-index`, `read-tree`, `update-ref`, `update-index`, `gc` and `filter-branch`, in any
  form (`push --dry-run` included);
- `apply`, except with only `--check`, `--stat`, `--numstat` or `--summary`;
- `add`, `mv` and `rm`, except with `-n`/`--dry-run`;
- `stash`, except `stash list` and `stash show`;
- `checkout` with a pathspec (`.`, anything after `--`, a second positional,
  `--pathspec-from-file`, `--ours`/`--theirs`, `-2`/`-3`), or with `--force`/`-f` or
  `--patch`/`-p`;
- `switch` with `--force`/`-f`, `--discard-changes`, `--force-create`/`-C` or `--orphan`;
- `clean`, except with `-n`/`--dry-run` (before `--`, and not as the pattern of `-e`);
- `branch` and `tag` in any but their list forms (`branch -d/-D/-m`, `branch <name>`,
  `tag -a`, `tag <name>` are writes; `branch -a`, `--list`, `--contains`, `tag -l`, `tag -n` are
  not);
- `bisect`, except `bisect log`, `bisect view` and `bisect visualize`;
- `worktree add`, `remove`, `move` and `prune`;
- `submodule update`, `deinit`, `add` and `sync`;
- `sparse-checkout`, except `sparse-checkout list`;
- `reflog expire` and `reflog delete`;
- `notes add`, `remove`, `edit`, `append`, `copy` and `prune` (also after `--ref <ref>`);
- `replace`, except its list forms (`git replace`, `-l`, `--list`).

Long options of `checkout`, `switch` and `clean` are read by their unique prefix, as git reads
them: `--forc` is `--force`, `--disc` is `--discard-changes`, `--dry` is `--dry-run`. An ambiguous
prefix (`checkout --pat`) is refused by git itself. Any other git command passes; see
[Known gaps](#known-gaps).

It can still read, search, build, check and run tests, and run `git status`, `diff`, `log`, `show`,
`blame`, `fetch`, `checkout <branch>`, `checkout -b <new>`, `switch <branch>` and
`switch -c <new>`.

A tagged agent's Bash line that the reader cannot follow still meets a coarse pattern: a `git`
word followed by `commit`, `reset`, `restore`, `merge`, `rebase`, `cherry-pick`, `revert`, `am`,
`apply`, `add`, `rm`, `mv`, `pull`, `push`, `clean`, one of the plumbing writes above, `stash` (but
`list`/`show`), or a writing action of `bisect`, `worktree`, `submodule`, `sparse-checkout`,
`reflog` or `notes`, is refused. Being coarse, it also refuses the dry runs of `apply`, `add`, `mv`,
`rm` and `clean` on such a line. Anyone else's unreadable line stays allowed.

**Workflow agents.** A workflow script's `agent()` raises the same spawn event, and its tool calls
carry the same `agentId` (verified live, see below). Its spawn names no parent agent, though, only
the `tool_use_id` of the `Workflow` call that started its run. So the mod notes the `tool_use_id` of
each `Workflow` and `Agent` call a tagged agent makes, before the call runs, and tags a workflow (or
nested) spawn under a noted call. This path is tested through the test kit only; see
[Known gaps](#known-gaps).

### G5: Fable subagents

G5 reads the Agent tool's `model` parameter and refuses any value matching `fable` in any case
(`fable`, `claude-fable-…`). A refusal has to come before the agent starts, and only that parameter
is known by then. See [Known gaps](#known-gaps) for what that misses.

### G6: `--no-verify`

G6 refuses `git commit` with `--no-verify`, an abbreviation of it (git 2.43 takes `--no-veri`;
`--no-ver` is ambiguous with `--no-verbose`), or `-n`, also inside a cluster of short flags (`-an`,
`-nm msg`). A later `--verify` cancels it.

A value is not a flag: `-m -n` and `-mn` are messages, `-Cn` names a commit, and an abbreviated
value-taking option takes the next word as git does (`--mess -n` is a message). Git's own options
before `commit` (`-C dir`, `-c k=v`, `--git-dir`, …) are skipped. Other commands
(`git log -n 3`, `git push --no-verify`) are not checked. The refusal shows the commit with the
flag taken out (`-an -m x` becomes `-a -m x`).

## What it never does

- **It never rewrites a call.** Every guard answers with a refusal or lets the call through
  unchanged. (In auto mode a rewritten call is denied outright, ANA-29 §2.3.)
- **It fails open.** If a guard throws, or the tag store cannot be read or written, the call goes
  through and the status line says which guard failed, for example
  `htui-dev: read-only guard failed, call allowed`. A tag store that holds no list counts as
  unreadable. If the spawn hook fails after the agent started, the status line says the read-only
  tag may be missing. A command the parser cannot read is let through silently (but see G4).
- **It reaches nothing outside the event.** No process, file, network or model call. The only
  things it calls are `$.state` for the read-only tags (`readOnly`, the tagged agents' ids, and
  `readOnlyCalls`, the `tool_use_id`s of their `Workflow` and `Agent` calls) and `$.ui.status` for
  those warnings. The main loop's tool calls never read the state. `claude plugin validate` lists
  exactly `$.state.get, $.state.set, $.ui.status`.

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
  hooks/register.ts            the four hooks, the fail-open helpers, the readOnly/readOnlyCalls state
  hooks/guards.ts              the pure decisions, the G1 feature tables, the refusal texts
  hooks/shell.ts               the Bash command reader
  types/index.d.ts             the $.state contract (htui-dev.readOnly, htui-dev.readOnlyCalls)
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
engine. The kit passes a call's `agentId` and a spawn's `parentAgentId`, `workflow` and
`tool_use_id` through to the mod, so the read-only tests run through the real hooks. The parser, the
git table and the spawn hook's backstop are also tested directly. The kit refuses a spawn answer of
the wrong shape (a `deny` with other fields beside it), so the spawn hook's pass-through of a refusal
from below is only checked for the plain `{ deny }` shape.

For type-checking, use the `tsconfig.json` from the header of the `claude-code.d.ts` that
Claude Code writes, kept **outside** the mod folder (`.claude-plugin/types/` is regenerated on every
load).

Two rules for anything under `.claude/skills/htui-dev/`:

- Never spell the forbidden literal. Build it as `'ze' + 'ta'`, as `guards.ts` and the test fixtures
  do. Nothing here is under `crates/`, so the scan never reads it, but a copied line could end up
  there.
- Keep the G1 tables in `guards.ts` in step with the manifests: `FEATURES` (each crate's
  `[features]` and their forwards), `SELECTING` (the features each crate's `[dependencies]` and
  `[dev-dependencies]` ask for) and `GATES` (the gated test files). `cargo tree -p <crate> -e
  features,normal,dev` shows what cargo really turns on. Nothing checks this until TOOL-9 adds the
  mod's tests and validator to `scripts/gate.sh`; its drift check should compare against
  `cargo tree -e features`.

## Known gaps

- **G5 sees only an explicit model.** A subagent with no `model` parameter takes its model from its
  agent definition, then from the parent, and that happens after the hook could refuse it. A
  refusal after the agent starts would fail the hook. So a Fable model pinned in an agent definition,
  or inherited from a Fable session, starts anyway, and the mod only shows a status line:
  `htui-dev: subagent <id> resolved to Fable (<model>) — inherited or pinned in its agent definition`.
- **G1 follows the working directory only within one Bash line.** A `cd crates/<member>` earlier in
  the line is read (see [G1](#g1-and-g2-cargo-test)), but the folder the line starts in is not
  known: the session's own, or one an earlier Bash call moved to. It is read as the workspace
  root. So a bare `cargo test` run from `crates/htui-store` that way is refused with `htui`'s fix,
  although cargo would test only `htui-store` there; use `-p` from the root, or `cd` in the same
  line. `cd -`, `cd ~`, a `$VAR` and `popd` are read as the root too. A `cd` inside `( … )` or a
  `bash -c` string is read as if it outlasted them, so in `(cd crates/htui-store && cargo test);
  cargo test` the second run is let through. A relative `--manifest-path` is read from the root. A
  `cargo test` in another project from a session opened here is read the same way.
- **The reader is not a shell.** It does not follow `eval`, scripts, aliases, functions, `sudo`,
  `xargs`, a command piped into a shell (`cat <<EOF | sh`, `… | bash`), or backticks inside double
  quotes, and it treats every `$VAR` as text. A `case … esac` inside a `$( … )` is misread: a
  pattern's `)` ends the substitution early, so the rest of that arm goes unchecked (nothing is
  refused by mistake). A guard can be walked around on purpose. It is there
  to catch mistakes.
- **G4 refuses only the git commands it lists.** Any other git command passes from a read-only
  agent, for example `notes merge`, `remote add`, `config`, `symbolic-ref`, `prune`, `repack`,
  `pack-refs`, `fast-import`, `lfs`, `maintenance`, `worktree lock`, and `submodule foreach`, which
  runs a command of its own. `checkout -m` and `switch -m` carry local changes into the new branch
  and are allowed.
- **Serena's `execute_shell_command` is not matched.** Serena's `claude-code` context excludes it.
  A Serena started with another context would offer it, and the commands it runs never meet the
  Bash guards.
- **G4 does not refuse `git checkout <one path>`.** `git checkout src/a.rs` restores a file, but
  with one positional the reader cannot tell a path from a branch name, so it is allowed. `.`, a
  second positional and anything after `--` are refused.
- **Workflow agents started by a read-only agent are tagged through the test kit only.** The live
  probe saw a main-loop workflow spawn carry no `parentAgentId`. The types say a workflow agent's
  `tool_use_id` is that of the Workflow call that started its run, and the mod tags by it, but no
  live run has yet shown a read-only agent's workflow being tagged.
- **G4's tag lands as the spawn returns.** The tag is written as soon as the engine answers the
  spawn with the agent's id, before that agent's first model turn returns. The live probe saw a
  10-second gap before the first tool call.
- **The field names of Gortex's compact `edit` and `refactor` tools are assumed.** The session that
  built this exposed only the older names (`edit_file`, `write_file`, `edit_symbol`,
  `rename_symbol`, `batch_edit`), whose fields were checked. The compact tools are matched by name
  and read with the same fields, plus `target.file` and `target.symbol`.
- **A rename is checked at the symbol's own file.** Gortex's and Serena's `rename_symbol` write the
  new name into every file that refers to the symbol, but G3 judges only the file the call names.

## What the live probe showed

TOOL-8's Task 0 ran a throwaway probe mod against Claude Code 2.1.296 under `claude -p`, with a
scratch configuration:

- A skills-directory plugin loads with no prompt beyond workspace trust (`hooks module
  probe@skills-dir loaded`).
- A plain subagent's `agentId` comes back from the spawn before its first tool call, and that tool
  call carries the same id.
- Workflow-tool agents raise the spawn event (`subagentType` `workflow-subagent`, a `workflow`
  field, no `parentAgentId`), and their tool calls carry the `agentId` the spawn answered. So G4
  covers a workflow agent tagged by its own prompt or type.
- The Workflow tool is available under `-p`. A workflow agent's prompt and `model` option reach the
  hook as given (`"READ-ONLY verifier. …"`, `model=sonnet`).

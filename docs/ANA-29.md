# ANA-29 - Claude Code mods: developing htui, and using htui in other projects

> **Scope note:** "Claude Code mods: developing htui, and using htui in other projects. Research
> how to build 'mods' for Claude Code (plugins, skills, agents, hooks, rules, MCP config, settings)
> in two directions. (a) Dev-side: what htui's own development should vendor in `.claude/` beyond
> today's `handoff-*` and `plan*` skills, `code-architect`/`rust-reviewer` agents and Gortex
> permissions, and what is better packaged as a Claude Code plugin. (b) Consumer-side: what htui
> ships so other projects can drive it from Claude Code (the `docs/htui-mcp.md` MCP surface, a
> plugin/skill bundle, install and update path, per-project config). Conclude with a recommended
> shape, a split between the two, and the MOD items to file. No code before the verdict."
> (`HANDOFF.md:54-61`, ANA-29.)
>
> **Maintainer clarification (2026-10-08, at routing):** "mods" means the Claude Code **mods
> feature** (https://claude.com/it/resources/articles/claude-code-mods,
> https://code.claude.com/docs/en/plugins/mods/overview): in-process JS/TS event handlers shipped
> in plugins. It does not mean the generic list in the item's parenthetical. And "include the
> plugins too". Mods are the centre of this analysis. Plugins are in scope as the vehicle, with the
> skills, agents, settings hooks and MCP servers they carry.
>
> **Requirements addressed:** `R-ID-2`, `R-ID-3`, `R-ID-4`, `R-ID-5`, `R-ID-6`, `R-ID-7`,
> `R-STO-1`, `R-AGT-1`, `R-AGT-4`, `R-ENT-8`, `R-HIS-1`, `R-MCP-1`, `R-MCP-3`, `R-MCP-4`,
> `R-TUI-4`, `R-TUI-6`, `R-TUI-11`, `R-NF-1`, `R-NF-2`.
>
> **Status (2026-10-08): concluded.**
> Verdict: **three small layers, and one fix that comes first.**
> - **First, a latent hazard.** htui starts `claude -p` without `--bare` (D92) and
>   passes no other isolation flag, so every session htui drives loads the user's plugins, skills,
>   settings hooks and mods. No user mod is installed today (§4.1), but any mod installed later can
>   approve a tool call before htui's permission relay sees it, and `R-ID-5`'s "agents run with
>   their own system skills disabled" is not enforced on the `claude-cli` path. **MOD-91** turns the
>   user's mods, settings hooks and skills off in htui-driven sessions, after a live probe of the
>   levers (`--settings '{"disableAllHooks":true}'`, `--disable-slash-commands`) against
>   subscription auth and htui's own `--mcp-config`. Plugin agents and plugin MCP servers stay
>   loaded under those levers (§2.6, §5.5).
> - **Dev side.** `.claude/` stays as it is: rules, workflow skills, agents and settings. Moving
>   them into a plugin would rename them and gain nothing. The new piece is one **`htui-dev` mod**,
>   committed as a skills-directory plugin at `.claude/skills/htui-dev/`. It loads in place in
>   every checkout and sandbox clone after workspace trust, with no install and no version bump.
>   It holds guards for the mistakes that today live only in memory notes (vacuous test greens,
>   the forbidden `zeta`, read-only verifiers that commit, Fable staffing), an ambient status line
>   (disk, Postgres, merge and push state, item and phase) and turnless commands (`/gate`,
>   `/next`, `/validate`). Logic stays in `scripts/` with `.ps1` twins. The mod launches and
>   displays (**TOOL-8**, **TOOL-9**).
> - **Consumer side.** A plugin cannot reuse htui's MCP server: it is token-bound to one session
>   htui launched. The base layer is agent-neutral: a **read-only `--json` CLI** (`htui status`,
>   `htui waiting`, `--search-items --json`) that any agent can call (**MOD-92**). On top sits a
>   thin **`htui` plugin**: one skill and one mod (a waiting-on-you band and pane, `/htui`
>   commands). `htui claude emit` writes it from the binary into a local directory marketplace, so
>   plugin and binary never drift (**MOD-93**). Answering relayed permissions from Claude Code is a
>   separate item behind a maintainer decision (**MOD-94**).
> - **Out:** one plugin for both audiences, a consumer MCP server in v1, a bridge to htui's socket,
>   any LLM call in a mod's decision path (`R-ID-6`), a mod that holds the DSN (`R-STO-1`), and a
>   hosted git marketplace.
>
> Eight items filed on 2026-10-08 (maintainer chose all of them): MOD-91 to MOD-94, TOOL-8 to
> TOOL-10, CLEAN-12, plus an amendment to MOD-16.
> One requirement clarification is proposed (`R-ID-5`, §10).

Code citations are against HEAD `46b570c1`.

**Sources.** Docs were fetched on 2026-10-08, when the reference table was stamped v2.1.290. The
local CLI is `2.1.294 (Claude Code)` (`claude --version`), which is past the mods floor. Short
forms below:
- `mods/<page>` is `https://code.claude.com/docs/en/plugins/mods/<page>` (overview, create,
  reference, events, api, interface, test, troubleshoot, admin).
- `plugins/<page>` is `https://code.claude.com/docs/en/plugins/<page>` (manifest-reference,
  components, loading, install, org, marketplace-reference, host-marketplace, publish).
- `settings-reference` is `https://code.claude.com/docs/en/settings-reference`.
- `cc-types:<line>` is the type file Claude Code 2.1.294 writes into a mod's
  `.claude-plugin/types/claude-code/index.d.ts` (header `// Written by Claude Code 2.1.294.`).
  It was taken from a throwaway probe mod written outside the repo.
- `cc-mods` is https://github.com/anthropics/claude-code/tree/main/mods (the built-in mods'
  source).
- `memory:<name>` is `~/.claude/projects/-home-mluigi-projects-htui/memory/<name>.md`.

No third-party mod code was run. The probe mod was a two-hook mod written for this analysis.

---

## 1. Context and problem statement

htui has a Claude Code surface on two sides.

- **Developing htui.** The repo carries `.claude/` with rules, workflow skills, two agents and
  Gortex permissions. Many working rules live only as memory notes that every agent prompt has to
  restate, and the evidence shows that this fails silently (§4.2).
- **Using htui elsewhere.** htui drives Claude Code as one of its agents. Nothing lets a person
  working in plain `claude` in another project see or act on htui's state.

Claude Code shipped **mods** on 2026-10-01: JS/TS event handlers that run inside Claude Code,
ship in a plugin, and can guard tool calls, draw panes and bands, and add commands that need no
model turn (`mods/overview`; https://github.com/anthropics/claude-code/issues/91870). The
announcement says "We plan to move more built-in features to mods over time"
(https://claude.com/it/resources/articles/claude-code-mods). The questions:

1. What can a mod do, and what does that change for each direction (§2, §3)?
2. What exists today on each side (§4)?
3. For each idea, is a mod the right mechanism, or a settings hook, a skill, an MCP server or a
   script (§5)?
4. What shape to build, how to split it, and what to file (§6 to §8)?

## 2. What mods are, and what matters here

### 2.1 Definition and versions

- A mod is a plugin with a **hooks module**. Its event handlers run inside Claude Code's process.
  They can observe, rewrite or answer an event (`mods/overview`, "How a mod works").
- Mods are on by default from CLI v2.1.287 and Desktop v2.1.286. The early-access flag
  `CLAUDE_CODE_ENABLE_FUNCTION_HOOKS` is ignored from 2.1.287 on (`mods/overview`, "Turn mods on or
  off").
- The API is young. The built-in mods' README says "the API these mods are written against may
  change between releases without notice" (`cc-mods` `README.md:115-116`). A community scan found
  21 of 359 mods failing validation on 2.1.287 (https://karanbansal.in/blog/claude-mods-scoreboard/).

### 2.2 Shape of a mod

- **Files** (`mods/reference`, "Files"):
  - `.claude-plugin/plugin.json`, with no mod-specific fields;
  - `hooks/hooks.json` with `"modules": ["./register.ts"]`, exactly one path;
  - the module itself, `.js`/`.ts` and friends, exporting `register(on, options)`. `options`
    carries the plugin's `userConfig` values;
  - optionally `types/index.d.ts` and `*.test.ts`.
- No Node, bundler or build step. `.ts` loads directly (`mods/create`).
- The module has no Node APIs, no file or network access of its own, and no `setTimeout`. Every
  effect goes through the `$` API (`mods/api`). That is why `claude plugin validate` can list
  what a mod does without running it (`mods/overview`, "List what a mod does").
- A hook is `on(event, matcher?, async ($, e, next) => …)`. Returning `next(e)` observes. Calling
  `next` with a changed event rewrites. Returning a result without calling `next` answers, for
  example `{deny: '…'}` on `tool.call` (`mods/events`).
- A hook that throws before calling `next` is skipped: the chain **fails open** unless the hook
  adds `.catch` (`mods/events`, "When a hook fails").

### 2.3 Events and API that matter for htui

| Need | Event or API | Note |
|---|---|---|
| Refuse or annotate a tool call | `tool.call` with a matcher such as `{tool: 'Bash'}` | Fires for subagent and MCP calls too. Each event carries the `agentId` of its loop (`cc-types:114-140`, `:192-203`) |
| Approve or deny before the prompt | `tool.check` → `{decision: allow\|ask\|deny}` | Runs after non-managed `PreToolUse` hooks and can approve a call they blocked (`mods/admin`, "Know what happens by default") |
| Govern subagents | `agent.spawn` | Can choose the model or `{deny}` the spawn (`mods/reference`, "Subagents") |
| Commands with no model turn | `$.command.register({name, immediate: true})` plus a `command.run` hook returning `{text}` or `{}` | Works while Claude is busy (`mods/api`) |
| Ambient state | `$.ui.status`, `$.ui.toast`, `$.ui.log`, the `AbovePrompt` band, a Pane via `$.ui.open` | A pane opened unprompted is placed only from 144 columns, so a band or status line is the fallback (`mods/interface`) |
| Timers | `$.clock.every` | Timers die on reload |
| Run a program | `$.process.run(argv, {timeoutMs})`, `$.process.spawn` | `run`: 30 s default, 10 min max, 4 MiB per stream (`mods/reference`, "Limits"). `spawn` stdin is "A string today; a later form may take the text in pieces" (`cc-types:8047`), so no two-way JSON-RPC to a child |
| Call MCP | `$.mcp.call(server, tool, args)`; `$.mcp.connect` only for servers in the mod's own plugin | The types say "No permission prompt" (`cc-types:2674`). The admin page says "under the session's permission rules" (`mods/admin`). The conflict is **UNVERIFIED** at runtime |
| HTTP | `$.http.fetch(url, {…, socketPath})` | HTTP only, whole body only, no streaming (`cc-types:3451-3459`) |
| A model call | `$.model.complete`, `fork`, `classify` | Spends the user's plan. Banned in any htui decision path (`R-ID-6`, §6.3) |
| Platform | none | There is no platform field in `$` (no `platform` or `win32` in `cc-types`). A mod picks `.sh` or `.ps1` by reading an environment variable such as `OS` through `$.env.get` (**UNVERIFIED**) |

In auto mode, a hook that changes a tool call's input after the server-side classifier reviewed it
gets the call **denied** ("a hook changed this call's input after the model wrote it",
`mods/troubleshoot`, "A tool call is denied"). So a guard should deny with a reason, never
rewrite.

### 2.4 Where mods run

From `mods/overview`, "Where mods run":

| Where | Hooks run | Drawing shows |
|---|---|---|
| `claude` in a terminal | yes | yes |
| Desktop Code tab (not WSL) | yes | yes, except terminal-only elements |
| Desktop WSL session | no (no plugins) | no |
| VS Code chat panel | yes | no |
| `claude -p` and the Agent SDK | **yes** | no |
| Cloud session | yes, if the plugin reaches it | no |

Two consequences for htui:
- A mod is never a UI **inside** a session htui drives. htui runs `claude -p` or ACP, where
  nothing draws.
- A mod's hooks **do run** inside those sessions if the user installed it (§4.4).

### 2.5 Limits that shape a design

- 10 s of a hook's own CPU time per event. Time spent awaiting `next` or a `$` call does not
  count, except `$.clock.sleep` (`mods/reference`, "Limits").
- All installed mods share one worker thread. A blocking hook gets its mod unloaded. Three
  untraced crashes turn off every non-built-in mod for the session (`mods/troubleshoot`, "it crashed
  the hooks worker").
- `$.process.run` tops out at 10 min. htui's full gate runs longer than that, so a mod cannot hold
  a gate run in a hook. It has to start a detached script and poll a status file.
- `$.store` is 4 MiB, machine-wide, shared by every session, with no atomic read-modify-write
  (`mods/interface`). It may cache. It never owns state (`R-ID-3`).

### 2.6 Security model

- **Unsandboxed.** A mod runs as the user. It can read secrets, see and rewrite every prompt and
  tool call, approve calls before the user is asked, and spend the user's plan. OS sandboxing
  covers Claude's Bash only. A process a mod starts runs outside it (`mods/overview`, "What a mod
  can reach").
- "A mod that approves tool calls can approve one that an `ask` rule would prompt for, or that one
  of your own `PreToolUse` hooks blocked" (same section). In auto mode, "a call the mod approves
  runs without a classifier check" (`mods/admin`).
- **`sec-default@builtin`** loads first only on machines with managed settings or for Team and
  Enterprise users. It shields managed hooks, the system prompt and managed MCP tools, and stops a
  user's mod from lifting `deny` rules (`mods/admin`; `cc-mods`
  `sec-default/hooks/register.ts:24-106`). On a personal machine without managed settings it is not
  there. `deny` rules never cover a mod's own `$.fs` or `$.process` calls (`mods/admin`).
- **Off switches:** `--safe-mode` for one session, `"disableAllHooks": true` in settings, and the
  managed `allowManagedModsOnly`. `disableAllHooks` stops a mod but leaves its plugin's skills,
  agents and MCP servers loaded (`mods/overview`, "Turn mods on or off"; `mods/reference`,
  "Settings": `disableAllHooks` applies from "Any settings file").

### 2.7 Development loop and checks

- `claude --plugin-dir <dir>` loads a plugin for one session and reloads it on save (`claude
  --help`; `mods/troubleshoot`).
- Claude Code writes the type files each time it loads or reloads a mod from a `--plugin-dir`
  directory, or a mod Claude wrote (`mods/create`, "Get type definitions for your version";
  §Sources). The probe confirmed this for 2.1.294. Whether it also writes them for a
  marketplace-installed or skills-directory plugin is **UNVERIFIED**.
- `claude plugin validate [--strict]` lists `hooks:` and `calls:` without running the mod.
  `claude plugin test` runs `*.test.ts` with no session, sign-in or network, using
  `claude-code/testing` stubs (`mods/test`; `claude plugin --help`). Both run in a gate.

## 3. Plugins as the vehicle

### 3.1 Components

A plugin is "a directory of skills, agents, hooks, MCP servers, or other components that Claude
Code installs and loads as one unit" (`plugins/manifest-reference`).
- Defaults: `skills/<name>/SKILL.md`, `agents/*.md`, `hooks/hooks.json` (settings hooks and the mod
  entry), `.mcp.json`, `bin/` (put on the Bash tool's PATH), `settings.json` (only `agent` and
  `subagentStatusLine` take effect).
- **There is no rules component.** `.claude/rules/` cannot move into a plugin.
- **Everything is namespaced.** Agent `rust-reviewer` in plugin `htui-dev` becomes
  `htui-dev:rust-reviewer`, and skills run as `/htui-dev:plan`. A plugin's MCP tools become
  `mcp__plugin_<plugin>_<server>__<tool>`, which permission rules written as `mcp__<server>__*` do
  not match (`plugins/components`; https://code.claude.com/docs/en/mcp).
- Plugin agents ignore `permissionMode`, `hooks`, `mcpServers` and `initialPrompt`
  (`plugins/components`).
- `userConfig` fields (`string|number|boolean|directory|file`, with `sensitive` going to the OS
  credential store) reach a mod as `register(on, options)`. `claude plugin install` never prompts
  for them; `/plugin configure` or `--config KEY=VALUE` does (`plugins/manifest-reference`, "User
  configuration").
- `${CLAUDE_PLUGIN_ROOT}` changes on every update. `${CLAUDE_PLUGIN_DATA}` survives updates
  (`plugins/manifest-reference`, "Environment variables").

### 3.2 Ways a plugin reaches a session

| Route | Install step | Updates | Notes |
|---|---|---|---|
| `--plugin-dir <dir>` | none, per launch | live, reload on save | Needs the flag every time. The mod-author's loop |
| **Skills-directory plugin**: a folder with `.claude-plugin/plugin.json` under the project's `.claude/skills/` | none, after workspace trust | the checkout's current files | Loads as `<name>@skills-dir`, only from the primary working directory (`plugins/loading`; `plugins/org`, "Keep skills-directory plugins loading") |
| Project `extraKnownMarketplaces` + `enabledPlugins` with a `directory` source | none for relative-path plugins | the marketplace copy's current files | A relative `directory` path resolves against the **main checkout, even from a git worktree** (`plugins/org`, "Require plugins per repository"). Ignored in untrusted folders "without a message" |
| User-scope directory marketplace (`claude plugin marketplace add <dir>`) | one `install` | the directory's current files at session start | No version bump needed for relative-path plugins (`plugins/loading`, "Versions and updates") |
| Git or URL marketplace | `marketplace add` + `install` | `claude plugin update`; third-party auto-update is **off** by default | A pinned `version` freezes users until bumped (`plugins/loading`; `plugins/install`) |
| `command` source | accept the command | re-run once per session | 2.1.229+; link mode refused on Windows (`plugins/marketplace-reference`) |
| claude.ai directory | portal submission | automatic | Needs a paid plan and a **GitHub** repo. `bin/` plugins are refused there (`plugins/publish`) |

### 3.3 What reaches the hr sandbox and cloud sessions

- **hr sandbox.** The clone sits at the host repo's path, so the host's trust flag applies.
  `~/.claude` is bind-mounted **read-write and shared**: login, settings, plugins, skills
  (`docker/hr/compose.hr.yaml:99-114`). Only transcripts and per-session directories are masked
  (`compose.hr.yaml:115-132`); `~/.claude/plugins` and `~/.claude/dev-mods` are not. A
  skills-directory plugin in the clone loads the **sandbox branch's** copy.
- **Cloud sessions.** The repo's `.claude/skills|agents|rules`, settings hooks and `.mcp.json`
  carry over. Plugins the repo enables do not: "A cloud session doesn't install the plugins a
  repository turns on" (https://code.claude.com/docs/en/cloud-environments). Whether a project
  skills-directory plugin loads there is **UNVERIFIED**; cloud never shows the trust dialog.

## 4. Current state

### 4.1 Dev side

| Artifact | State |
|---|---|
| `.claude/settings.json` | Only `mcp__gortex__*` allow rules. No hooks, no `enabledPlugins` |
| `.claude/settings.local.json` | Not committed (`.git/info/exclude`). Holds the Gortex hooks. `scripts/hr up` copies it into a sandbox (`scripts/hr:826-827`) |
| `.claude/agents/code-architect.md`, `rust-reviewer.md` | Committed, no `model:` pin, Gortex read tools in `tools:` (`memory:code-architect-cannot-read-source`) |
| `.claude/rules/workflow-docs.md`, `concept-docs.md` | Committed. No plugin can carry them (§3.1) |
| `.claude/skills/handoff-run`, `handoff-add`, `plan`, `plan-prd`, `handoff-docs.md` | Committed, with `.sh`/`.ps1` script twins. htui's copy leads the synced surface (`.claude/skills/handoff-run/references/ecosystem-survey.md:99-113`, "htui revisit"; `memory:htui-workflow-skills-lead`) |
| `.claude/skills/graphify`, `.claude/skills/generated/gortex-*` (20 files) | Committed. `scripts/hr:692` lists `/.claude/skills/generated/` as tool-owned and untracked, which contradicts the commit |
| `.claude/workflow-config.json` | `{"reviewer":"rust-reviewer"}` |
| `.claude/.headroom_wrap_settings.lock` | A zero-byte tool lock file, committed (`git ls-files .claude`) |
| Git hooks | `.git/hooks` holds only `*.sample` files and `core.hooksPath` is unset. Yet `.claude/skills/handoff-run/references/lifecycle.md:98-99` says the validator "also runs automatically at commit time (`pre-commit` hook)". `install-workflow-hooks.sh:22-23` relies on "a separate PreToolUse hook [that] hard-blocks `--no-verify`"; no settings file here has one |
| Mods | None installed. `enabledPlugins` at user scope has `remember` and `superpowers` only |

### 4.2 Dev friction a mod could remove

Ranked by cost. Each row has evidence of a real loss.

| # | Friction | Evidence | Fit |
|---|---|---|---|
| D1 | Vacuous greens: `cargo test -p htui` without `--features testkit` runs 0 integration tests and reports ok; other crates name the feature `test-support` | `memory:htui-integration-tests-need-testkit` | Guard on `Bash`, plus a canonical gate script |
| D2 | The forbidden literal `zeta`: per-crate gates are green, the workspace gate catches it lanes later | `crates/htui-agent/tests/extensibility.rs` (`the_codebase_has_never_heard_of_zeta`); `memory:htui-zeta-is-a-forbidden-literal` | Guard on `Edit`/`Write`/`MultiEdit` |
| D3 | A "read-only" verifier fixed and committed the defect it found | `memory:ultracode-scoped-to-implementers` | Best as a mod: `agent.spawn` tags the spawn from its prompt, `tool.call` denies writes by `agentId`. A settings `PreToolUse` hook also sees `agent_id` and `agent_type`, but not the spawn prompt |
| D4 | Model staffing: every agent inherits the session model, never Fable; a pinned `model: opus` died on 402s | `memory:handoff-run-model-staffing`, `memory:htui-opus-402-blocks-pinned-agents` | `agent.spawn` deny with a reason |
| D5 | Gates are trustworthy only with `--test-threads=1 --no-fail-fast` and a featureless clippy run; a stack overflow aborts the run with 0 failures counted | `memory:htui-suite-green-is-scheduling-dependent`, `memory:htui-orch-test-stack-headroom`, `memory:htui-featureless-clippy-gate` | A script carries the sequence. A mod launches it and shows the result |
| D6 | Dev Postgres crash loop from disk pressure | `memory:dev-postgres-crash-is-disk-pressure`; today `/` is 72% and the repo disk 81% (`df -h`) | Status line from a timer |
| D7 | HANDOFF.md and DECISIONS.md conflict on most merges | `.remember/` notes; `HANDOFF.md:26-30` (migration renumbering at merges) | Mostly git: a union merge for the DECISIONS index, a migration-number check in the gate |
| D8 | No commit-time validator and no `--no-verify` block in this repo | §4.1 | Committed git hooks, plus a mod guard on `git commit --no-verify` |
| D9 | Session state is invisible: which item and phase, merge in progress, unpushed work | `.remember/now.md` | Status line or band |
| D10 | Each workflow script costs a model turn plus context | `next-item.sh`, `validate-workflow-docs.sh` | Turnless commands |

Not mods: a push guard in sandboxes is redundant, because the sandbox remote has push disabled
(`docs/hr-sandbox.md:115`; `git remote -v` in a run shows `no-push-from-sandbox`). The Gortex hook blocking Grep on non-indexed text is a Gortex policy at
user level, not htui's to override (§9).

### 4.3 Consumer side

- **The MCP server is inward-facing and session-scoped.** htui mints a 256-bit token per session.
  The agent starts `<htui binary> mcp` with `HTUI_MCP_ADDR` and `HTUI_MCP_TOKEN`
  (`docs/htui-mcp.md:27-45`). The subcommand is "Started by the agent htui launched, never by hand"
  (`crates/htui/src/cli.rs`, `Command::Mcp`). The socket is a Unix socket or named pipe and speaks
  newline-delimited JSON-RPC, never TCP or HTTP (`docs/htui-mcp.md`, "The socket"). MOD-79 moved
  the token into a `0600` file passed as `--mcp-config=<path>` (`docs/decisions/mod/mod-79.md`).
  **A static plugin `.mcp.json` cannot carry that token**, and `$.http.fetch` cannot speak that
  protocol (§2.3).
- **The CLI has no machine-readable output.** Subcommands are `worker`, `provision` and `mcp`.
  Flags include `--search-items`, which prints text lines (`crates/htui/src/cli.rs`). There is no
  `status`, `waiting`, `queue` or `--json`.
- **The reads exist.** `ReadStore::waiting_candidates`
  (`crates/htui-core/src/store/traits.rs:316`) and `WaitingView`
  (`crates/htui-worker/src/views.rs:540-554`, built by `fn waiting` at `views.rs:590`) already
  compute `R-TUI-11`'s list.
- **A second answerer is allowed by design.** MOD-42's executor polls a `step_permission` row every
  1 s "until any store client answers it with a compare-and-set"
  (`docs/decisions/mod/mod-42.md:31-32`). Today only the TUI answers.
- **The DSN** lives in the OS keyring, "Never in `argv`, the environment, or a plaintext file"
  (`R-STO-1`). A mod that shells out to `htui` stays inside that rule. A mod that holds a DSN
  breaks it.
- **A consumer repo commits nothing htui-specific today**, and htui writes nothing into managed
  repos (`R-ID-4`).
- **Install** is `cargo install --git https://git.mluigi.it/htui.git htui --locked` (README). There
  are no release binaries and no self-update.

### 4.4 A latent hazard

- htui's `claude-cli` argv has no `--bare`. The test says "`--bare` is the seed's business and D92
  dropped it there; the supervisor never adds one" (`crates/htui-agent/tests/cli_driver.rs:133-134`).
  D92 dropped it because `--bare` breaks subscription auth (`crates/htui-agent/tests/cli_live.rs:923-1028`).
- No code or seed passes `--safe-mode`, `--setting-sources`, `--disable-slash-commands` or
  `disableAllHooks` (Gortex text search: 0 hits for each).
- So every htui-driven Claude Code session loads the user's plugins, skills, settings hooks and
  mods, in every isolation mode. Mods run under `-p` (§2.4).
- The managed repo's own `.claude/` settings hooks and skills-directory plugins load only in
  `local` and `shared_serialized` isolation, where the session's cwd is the checkout itself
  (`crates/htui-orch/src/isolate/real.rs:692-700`). In `worktree` and `copy` modes the session
  starts in `<scratch_root>/<run>/<step>/` with the repo in a child directory (`real.rs:727`,
  `:862`), and neither loads from a child directory. A project skills-directory plugin also needs
  that exact folder to have been trusted interactively: "Trusting a parent folder or running with
  `-p` isn't enough" (`plugins/loading`, "Plugins shared through a repository").
- Effects:
  - A user's mod can approve a tool call before htui's `--permission-prompt-tool
    mcp__htui__permission_prompt` is asked (§2.6; `docs/htui-mcp.md`, "Permission prompts"). That
    undercuts MOD-42's relay and the policy MOD-11 set.
  - `R-ID-5` says "Agents run with their own system skills disabled". The `claude-cli` path does not
    enforce it. `claude --help` documents `--disable-slash-commands` as "Disable all skills".
  - Dogfooding makes it concrete: an htui step on htui's own repo, in `local` or
    `shared_serialized` isolation and in a checkout the maintainer has already trusted, would load
    the `htui-dev` mod this analysis proposes. A user-scope mod loads in every mode.
- Whether `claude-agent-acp` (htui's ACP `claude`) loads user plugins depends on the adapter's SDK
  `settingSources`. **UNVERIFIED.**
- A seam already exists: an agent row's launch arguments and `cli` extra arguments reach the argv
  (`cli_driver.rs:98-104`, `"--row-arg"`, `"--x"`). A user can opt into a flag today, by hand.

### 4.5 Prior art

| Tool | Lesson |
|---|---|
| `code-modernization` (official marketplace) | The closest analogue: commands, agents and shell hooks, plus a `register.ts` mod with a progress pane and sign-off dialog (`~/.claude/plugins/marketplaces/claude-plugins-official/plugins/code-modernization/hooks/`). Its README says the plugin behaves the same with mods off. The mod is a view over durable artefacts |
| Built-in mods (`diff`, `sec-default`, `telemetry`, `agents-md`) | Anthropic writes features as mods with `claude plugin test` suites (`cc-mods` `README.md`) |
| `blast-radius` sample | Holds `tool.call` for a person by looping `$.process.run(['sleep','0.25'])`, since sleeping in `$` does not use the 10 s budget. Falls back to the band when no pane can be placed (https://github.com/anthropics/claude-code-playground/tree/main/claude-code/mods) |
| Community status mods (`gh-ci-status`, `cc-pr-tracker`) | The common recipe: `$.process.run([cli, …, '--json'])` on a timer (https://github.com/karanb192/awesome-claude-code-mods) |
| Gortex | `gortex plugin emit` writes the plugin from the binary with the binary's version (`gortex plugin emit --help`). One source of truth, no drift |
| Taskmaster | Dropped copying files into `.claude/commands` because "copies won't receive updates" (https://newreleases.io/project/github/eyaltoledano/claude-task-master/release/task-master-ai@0.29.0) |
| beads, Backlog.md | A CLI plus short instructions costs about 1-2k tokens, against 10-50k for MCP schemas (https://gastownhall.github.io/beads/integrations/claude-code; https://github.com/MrLesk/Backlog.md) |
| superpowers, remember | One `skills/` tree, a thin manifest per host (Claude, Codex, Gemini) (`~/.claude/plugins/cache/claude-plugins-official/superpowers/6.4.1/`) |
| OpenRig | Writes hooks and statusLine into the user's Claude config. ANA-27 rejected that (`docs/ANA-27.md` §5.2) |
| Serena | The official marketplace entry lags upstream, and its maintainer cannot fix it (https://github.com/anthropics/claude-plugins-official/issues/121). Own the entry |

## 5. Options

### 5.1 Mechanism per idea

Each idea could be a mod, a settings hook (shell command in `settings.json`), a skill (text the
model reads), an MCP server, or a script. The pick is the smallest that works.

| Idea | Mod | Settings hook | Skill | MCP | Script | Pick |
|---|---|---|---|---|---|---|
| D1 vacuous-green guard | deny with a corrective reason | possible, shell | already in memory text; fails | no | `scripts/gate.sh` is the real gate | **Script as truth, mod as net** |
| D2 `zeta` guard | deny on write | possible, needs `.sh` and `.ps1` | no | no | the test exists | **Mod** |
| D3 read-only verifier | `agent.spawn` + `tool.call` by `agentId` | `PreToolUse` by `agent_type`/`agent_id`, shell; cannot read the spawn prompt | prompt text; fails today | no | no | **Mod** |
| D4 staffing | `agent.spawn` deny Fable | `PreToolUse` on `Agent` input, shell | handoff-run text | no | no | **Mod** |
| D5 canonical gate | launcher and viewer | no | no | no | runs the sequence | **Script, plus a mod command** |
| D6, D9 ambient state | timer + status line or band | `statusLine` command (shell, every render) | no | no | no | **Mod** |
| D7 merge conflicts | no | no | no | no | `.gitattributes` + gate check | **Script** |
| D8 `--no-verify` | deny | possible | no | no | committed hooks | **Git hooks, plus a mod guard** |
| D10 turnless scripts | `$.command.register` | no | costs a turn | no | the scripts exist | **Mod** |
| C1 htui state in a consumer session | band and pane over a JSON CLI | `statusLine`, shell | the model runs the CLI on demand | a new user-scoped server | the CLI | **CLI + mod**, skill for the model |
| C2 search htui concepts | `$.tool.register` | no | skill + CLI | possible | the CLI | **Skill + CLI** |
| C3 answer relayed permissions | buttons | no | must not (no agent answers, ANA-27 T2) | must not | a CLI verb | **CLI verb + mod buttons**, after a decision |
| C4 heavy builds through htui's queue | advisory guard | no | no | no | an out-of-session queue client | **Reject** for now (§9) |
| C5 isolation of htui's own sessions | no | no | no | no | argv flags | **htui code** (MOD-91) |

A settings hook is a shell command. It needs a `.sh` and a `.ps1` twin for Windows parity
(`R-NF-1`) and cannot add a command. Inside a subagent its input carries `agent_id` and
`agent_type` (https://code.claude.com/docs/en/hooks, "Common input fields"), so it can tell agents
apart, but it never sees the spawn prompt. A mod is JS that Claude Code runs on every platform,
reads the spawn prompt in `agent.spawn`, and keeps every guard in one place. It reaches cloud sessions only if the plugin does (§3.3). That is acceptable for the dev
side, where the maintainer works in the terminal and in hr sandboxes.

### 5.2 Dev-side vehicle

| Option | For | Against |
|---|---|---|
| V1 Move skills and agents into an `htui-dev` plugin | One unit | Namespacing renames `/plan` to `/htui-dev:plan` and the agents to `htui-dev:*`, breaking `workflow-config.json` and handoff-run references. Plain `.claude/` reaches cloud sessions, plugins do not. Rules cannot move. Gains nothing |
| V2 Mod in a git marketplace, installed at user scope | Standard | Installed into the shared `~/.claude/plugins`. Cached by version, so every change needs a bump. Sandbox-shared cache (§3.3) |
| V3 Project `directory` marketplace + `enabledPlugins` | No install | A session started in a git worktree (for example `claude` opened in `target/wt/x`) loads the main checkout's copy, not its branch's. Subagent worktrees (Workflow `isolation: 'worktree'`, agents given a `dir`) run inside one session and see that session's copy under V3 and V5 alike |
| V4 `--plugin-dir` in an alias and in `scripts/hr attach` | Live reload, per branch | A flag on every launch. Easy to forget, and then the guards silently do not exist |
| **V5 Skills-directory plugin at `.claude/skills/htui-dev/`** | Loads in place after trust, in every checkout and sandbox clone a session starts in, with that checkout's files. Changes go through git review. No install, no bump, nothing in `~/.claude/plugins` | Primary working directory only. Shadowed by a same-named `~/.claude/skills/htui-dev`. Whether it asks for anything beyond trust is **UNVERIFIED**. Cloud: **UNVERIFIED** |

**Pick V5**, with V4 as the mod author's hot-reload loop while developing it.

### 5.3 Consumer-side surface

| Option | For | Against |
|---|---|---|
| S1 Bundle `htui mcp` in a plugin | Reuses MOD-11 | Impossible: it needs a session token minted in process, and its tools are run-scoped |
| S2 New user-scoped MCP server (`htui mcp --user`) | Typed tools for any MCP client | A second, wider MCP contract with its own auth story. 10-50k tokens of schemas per session. Overkill for reads |
| S3 Mod bridges to htui's socket | No new process | Protocol mismatch: newline JSON-RPC behind a token, not HTTP. Would need an HTTP listener in htui (`R-NF-2` pressure) |
| **S4 Read-only `--json` CLI, called by a mod and a skill** | Deterministic, no daemon, DSN stays in the keyring, any agent can call it from Bash, testable without Claude | Each poll is a process start plus a store query |
| S5 Long-running `htui watch --json` child via `$.process.spawn` | Push instead of poll | Dies on reload, needs a new streaming contract. Later, if polling proves too slow |

**Pick S4.** Every write stays behind htui's own code: the CLI verb does the compare-and-set and
the scrubbing, never the mod.

### 5.4 Consumer distribution

| Option | For | Against |
|---|---|---|
| U1 Git marketplace on `git.mluigi.it` | Standard `marketplace add` | Plugin and binary versions drift. Auto-update is off for third parties. Anonymous readability **UNVERIFIED** |
| U2 claude.ai directory | Auto-updates | Needs a paid plan and a portal submission, and the listing versions apart from the binary, so the two drift. A public `mluigi/htui` GitHub repo exists (`memory:htui-remotes-and-pgpass`), so GitHub is not the blocker |
| U3 `command` source printing a path | Tracks the binary; can sit in a local `directory` marketplace like U4 | Accept-command prompt at install. Link mode refused on Windows, and copy mode caps the directory at 256 MiB or 20,000 entries. The command re-runs once per session (`plugins/loading`, "When a command source re-runs") |
| **U4 `htui claude emit [--dir]` writes a local directory marketplace from files embedded in the binary** | Plugin and binary in lockstep. Relative-path plugin loads current files each session, no bump. Works offline. Nothing written into any repo (`R-ID-4`) | One manual `marketplace add` + `install` the first time. Re-run `emit` after upgrading htui. The mod shows a skew line when versions differ |

**Pick U4.** `emit` writes under htui's own data directory by default and prints the two `claude
plugin` commands. It does not run them and does not edit `~/.claude` settings itself, in line with
ANA-27's rejection of htui writing a harness's config (`docs/ANA-27.md` §5.2).

### 5.5 Isolation of htui-driven sessions (MOD-91)

| Lever | Stops user mods | Keeps subscription auth | Keeps htui's `--mcp-config` | Keeps user MCP servers | Notes |
|---|---|---|---|---|---|
| `--bare` | yes | **no** (D92) | yes | yes | Out |
| `--safe-mode` | yes | yes ("Auth … work normally", `claude --help`) | **UNVERIFIED**: it disables "MCP servers" | no | Also drops CLAUDE.md, skills, agents |
| `--setting-sources project,local` | only user-enabled plugins, **UNVERIFIED** | likely | yes | partly | The managed repo's own hooks and plugins still load |
| **`--settings '{"disableAllHooks":true}'`** | yes for installed mods, **UNVERIFIED** for skills-directory and `--plugin-dir` mods | expected (settings only) | yes | yes | Flag settings outrank user, project and local (`plugins/loading`). Also stops settings hooks and the status line (`settings-reference`, `disableAllHooks`) |
| `--disable-slash-commands` | no | expected | yes | yes | "Disable all skills": the `R-ID-5` half |
| `--strict-mcp-config` | no | yes | yes | no | Contradicts the recorded choice to keep user MCP servers (`docs/htui-mcp.md:33-35`) |

**Pick `disableAllHooks` through `--settings` plus `--disable-slash-commands`**, subject to a live
probe like D92's (§8, MOD-91).

## 6. Verdict

### 6.1 Recommended shape

1. **Fix the hazard first (MOD-91).** htui turns off the user's mods, settings hooks and skills in
   every Claude Code session it drives, by default, with a per-agent-row opt-out. Plugin agents and
   plugin MCP servers stay loaded under the chosen levers; the probe records them and whether
   another lever is wanted. It sets a marker variable (`HTUI_SESSION=1`) in the agent's
   environment, so a mod that is let back in can stand down.
2. **Dev side: `.claude/` unchanged, plus one mod.** The `htui-dev` mod is a skills-directory plugin
   at `.claude/skills/htui-dev/` (V5). It holds guards (D1-D4, D8), a status line (D6, D9) and
   turnless commands (D5, D10). Logic lives in `scripts/` with `.ps1` twins. The mod launches,
   refuses with a reason, and displays (TOOL-8). The gate script, committed git hooks and the
   DECISIONS merge rule come first, because they cover humans and every clone (TOOL-9).
3. **Consumer side: a CLI first, a thin plugin second.** A read-only `--json` CLI is the
   agent-neutral base (MOD-92). The `htui` plugin is one skill and one mod, emitted from the binary
   into a local directory marketplace (MOD-93). It works without the mod: the skill alone teaches
   any Claude Code version to call the CLI. The mod adds the band, pane and `/htui` commands.
4. **Permission answers from Claude Code** are a separate item behind a maintainer decision
   (MOD-94).

### 6.2 The split

| Layer | Holds | Audience | Reaches |
|---|---|---|---|
| Repo `.claude/` (unchanged) | rules, `handoff-*`/`plan*` skills, `code-architect`/`rust-reviewer`, `workflow-config.json`, Gortex permissions | htui contributors | every checkout, sandboxes, cloud sessions |
| `.claude/skills/htui-dev/` (new, skills-directory plugin) | the `htui-dev` mod and its tests. No skills, no agents | htui contributors on CLI ≥ 2.1.287 | every trusted checkout and sandbox clone, terminal and `-p` |
| `scripts/` (new: `gate.sh`/`.ps1`, `githooks/`) and `.gitattributes` | the gate sequence, commit hooks, merge rule | humans and agents, every platform | every clone |
| `htui` consumer plugin (new, emitted by `htui claude emit`) | skill `htui`, the `htui` mod, `userConfig` (`panel`, `poll_seconds`) | people using htui beside plain `claude` in any project | user scope, via a local directory marketplace |
| htui binary (new: `--json` reads, `claude emit`, isolation flags) | all logic, all writes | everyone | — |
| User level (not htui's) | dedupe the Gortex hooks between `~/.claude/settings.json` and `.claude/settings.local.json`; Gortex's block on non-indexed text | the maintainer | — |

### 6.3 Rules every htui mod follows

- **No model calls in a decision path.** No `$.model.*` to decide status, links, queue order or a
  permission answer (`R-ID-6`). A mod may draft text a person sends.
- **No DSN, no SQL.** Reads and writes go through the `htui` binary, which reads the keyring
  (`R-STO-1`) and scrubs (`R-ID-7`).
- **Deny with a reason; never rewrite** a tool call (§2.3, auto mode).
- **Never answer a prompt for the user.** A permission answer is a button a person presses (ANA-27
  T2, `docs/ANA-27.md` §5.1).
- **Reach stays at "runs processes".** No `$.http`, no `$.fs.write`, no `$.session.send`. The
  `claude plugin validate` `calls:` line is the check.
- **Fail open, visibly.** A guard that throws lets the call through and sets a `$.ui.status`
  warning. A dev guard is a safety net, not a permission system. The exception is any guard that
  protects a write outside the checkout, which adds `.catch` and fails closed.
- **Long work runs detached.** A command starts a script that writes a status file. The mod polls
  that file and never holds a hook beyond the 10 s budget.
- **Platform by environment.** `.ps1` on Windows, `.sh` elsewhere. Cross-platform binaries (`htui`,
  `git`, `cargo`) are called directly.
- **Gate the mod.** `claude plugin validate --strict` and `claude plugin test` run in the gate when
  `claude` ≥ 2.1.287 is present.

### 6.4 What stays out

- Rules, workflow skills and agents stay plain files in `.claude/`.
- No consumer MCP server in v1.
- No hosted marketplace and no claude.ai listing in v1.
- No out-of-session `command_run` client.
- Nothing htui writes into a managed repo or into `~/.claude` settings.

## 7. Phasing

| Step | Item | Depends on | Why this order |
|---|---|---|---|
| 1 | MOD-91 isolation | nothing; starts with a live probe | Highest impact: the exposure exists today, plugin or not, though no user mod is installed yet |
| 2 | TOOL-10 sandbox hardening | nothing | A sandbox can plant a mod that runs in the next host session today |
| 3 | TOOL-9 gate script, git hooks, merge rule | nothing | Covers humans and clones; TOOL-8's `/gate` launches it |
| 4 | CLEAN-12 `.claude/` hygiene | TOOL-9 for the lifecycle text, TOOL-8 M1 for the `--no-verify` wording; the lock file and the generated-skills decision are not blocked | Small |
| 5 | TOOL-8 M1 guards | MOD-91 (so htui's own `local`/`shared_serialized` steps in a trusted checkout of this repo do not load it) | The D1-D4 losses are the costliest |
| 6 | TOOL-8 M2 status line and commands | TOOL-9 | `/gate` needs the script |
| 7 | MOD-92 `--json` CLI | nothing | Independent of the dev side |
| 8 | MOD-93 consumer plugin | MOD-91, MOD-92 | The mod reads only MOD-92. It must not load in htui-driven sessions |
| 9 | MOD-94 permission answers outside the TUI | maintainer decision, MOD-92; buttons after MOD-93 | Extends who may answer |

## 8. Items to file

IDs were minted at close-out through `scripts/hr-mint` (leased, `.claude/rules/workflow-docs.md`,
"Item ID prefixes"). The bodies below are as filed in `HANDOFF.md`.

**MOD-91** (Next features):

> - [ ] **MOD-91 - Keep the user's mods, settings hooks and skills out of htui-driven sessions**
>   (from ANA-29, `docs/ANA-29.md` §4.4, §5.5). `R-ID-5`, `R-AGT-1`, `R-AGT-4`, `R-TUI-6`, `R-HIS-1`.
>   htui launches `claude -p` without `--bare` (D92: it breaks subscription auth) and with no other
>   isolation flag. Every htui-driven session therefore loads the user's plugins, skills, settings
>   hooks and **mods**. In `local` and `shared_serialized` isolation, from a checkout already
>   trusted interactively, it also loads the managed repo's own `.claude/` hooks and
>   skills-directory plugins; `worktree` and `copy` sessions start in a scratch directory above the
>   repo and do not (`crates/htui-orch/src/isolate/real.rs:692-700`, `:727`, `:862`). A mod can
>   approve a tool call before `mcp__htui__permission_prompt` is asked, and `R-ID-5`'s "system
>   skills disabled" is not enforced on `claude-cli`. First a live probe, shaped like
>   `cli_live.rs` case 1: install a test mod at user scope and put another in a project
>   skills-directory plugin of a checkout that was trusted interactively (or has
>   `hasTrustDialogAccepted` set), and use that checkout as cwd. Confirm both mods load without the
>   flags, then spawn `claude -p` with `--settings '{"disableAllHooks":true}'` and
>   `--disable-slash-commands`, and record that subscription auth still works, that htui's
>   `--mcp-config` server and permission-prompt tool still work, that neither mod loads
>   (`--debug-file`, "hooks module … loaded" absent), and that skills are gone. Record which plugin
>   agents and plugin MCP servers stay loaded (`disableAllHooks` leaves them), and whether another
>   lever, such as `enabledPlugins` overrides in the same `--settings` value, should remove them.
>   Also probe `--safe-mode` and `--setting-sources` for the record. Then make the passing flags the default
>   for every `claude-cli` session htui starts (engine steps, chat, promotion, help turns). Add an
>   agent-row setting to opt back in, shown in Settings > Agents, and set `HTUI_SESSION=1` in the
>   agent's environment. For ACP `claude`, establish whether `claude-agent-acp` loads user plugins
>   (its SDK `settingSources`) and close the gap the same way. User MCP servers stay
>   (`docs/htui-mcp.md:33-35`). Document the change in `docs/htui-mcp.md` and the README. Not
>   blocked.

**MOD-92** (Next features):

> - [ ] **MOD-92 - Read-only `--json` CLI for htui state** (from ANA-29, `docs/ANA-29.md` §5.3).
>   `R-TUI-11`, `R-ID-2`, `R-ID-6`, `R-STO-1`, `R-NF-2`. Nothing outside the TUI can read htui's
>   state: the CLI has no `status`, `waiting` or `--json`. Add read-only subcommands that print
>   one versioned JSON document and exit: `htui status --json` (store state, box, working and
>   waiting counts, as in the top bar), `htui waiting --json` (`R-TUI-11`'s rows through
>   `waiting_candidates`/`WaitingView`: reason, item key, run, step, age), and `--json` for
>   `--search-items`. Scope is the active workspace, or `--project <slug>`, or the project whose
>   registered repo contains `--cwd <path>`. The DSN comes from the keyring like every other verb.
>   An offline or unreachable store answers a typed `{"store":"offline"}` document and exit status
>   3, never stale data presented as live. No writes. The JSON schema is documented and pinned by
>   tests, because MOD-93 and other agents parse it. Not blocked.

**MOD-93** (Next features):

> - [ ] **MOD-93 - `htui` Claude Code plugin: one skill, one mod, emitted from the binary** (from
>   ANA-29, `docs/ANA-29.md` §6). `R-ID-4`, `R-ID-6`, `R-STO-1`, `R-TUI-11`, `R-NF-1`. For people
>   working in plain `claude` beside htui. `htui claude emit [--dir PATH]` writes a directory
>   marketplace (default under htui's data directory) holding plugin `htui` with the binary's
>   version, and prints the `claude plugin marketplace add` / `install` commands. It never edits
>   `~/.claude` and never writes into a repo. Contents: a skill `htui` (how to read htui state with
>   MOD-92's verbs and when to hand work to htui, not do it inline), and a mod with (1) a status line
>   or `AbovePrompt` band showing the waiting-on-you count from `htui waiting --json` on a
>   `$.clock.every` timer, (2) a `/htui` pane listing the rows (opened by command, so it places at
>   any width), (3) turnless `/htui status`, `/htui waiting`, `/htui search <q>`, (4) a skew line when
>   `htui --version` differs from the plugin's version, (5) standing down when `HTUI_SESSION` is set.
>   `userConfig`: `panel` (`auto|command|off`), `poll_seconds`. Rules from `docs/ANA-29.md` §6.3:
>   no `$.model`, no DSN, reach limited to processes, `claude plugin validate --strict` and `claude
>   plugin test` in the gate. `emit` omits the mod and says so when `claude --version` is below
>   2.1.287. Blocked on MOD-91 and MOD-92.

**MOD-94** (Next features):

> - [ ] **MOD-94 - Answer a relayed permission request from outside the TUI** (from ANA-29,
>   `docs/ANA-29.md` §5.1 C3). `R-TUI-4`, `R-TUI-6`, `R-HIS-1`, `R-ID-6`. MOD-42's executor waits for
>   "any store client" to answer a `step_permission` row by compare-and-set
>   (`docs/decisions/mod/mod-42.md:31-32`), but only the TUI answers. Add `htui permission list
>   --json` and `htui permission answer <request-id> <option>`, which run the same compare-and-set,
>   name the request they answer, refuse a request already answered or whose session is gone, and
>   record the answerer as a person on this box (ANA-27 T2). MOD-93's pane then gains one button per
>   option, pressed by a person; no mod or agent answers on its own. **Blocked on a maintainer
>   decision** (`docs/ANA-29.md` §12 Q2) and on MOD-92; the buttons land after MOD-93.

**TOOL-8** (Tooling findings):

> - [ ] **TOOL-8 - `htui-dev` mod: guards, status line and turnless commands** (from ANA-29,
>   `docs/ANA-29.md` §4.2, §6). A skills-directory plugin at `.claude/skills/htui-dev/`
>   (`.claude-plugin/plugin.json`, `hooks/hooks.json` → `register.ts`, `*.test.ts`), loaded in
>   place by every trusted checkout and sandbox clone; develop it with `--plugin-dir`. **M1 guards**,
>   each a `{deny}` with a corrective reason, never a rewrite: `cargo test` of `-p htui` or
>   `--workspace` without `--features testkit`/`--all-features`, and `--features testkit` on crates
>   that call it `test-support`; the substring `zeta` in Edit/Write/MultiEdit content written to
>   `crates/**/*.rs` or `crates/**/*.json`, except `crates/htui-agent/tests/extensibility.rs` (the
>   test's own scope: `tree_files()` and `text.contains("zeta")`); Edit/Write and
>   `git commit|stash|reset|checkout --` from an agent whose spawn prompt declared it read-only (`agent.spawn` records the `agentId`);
>   an `agent.spawn` naming a Fable model (deny, not warn: maintainer, 2026-10-08); `git commit --no-verify`. Guards fail open with a
>   status warning. **M2 status and commands**: a status line from a 60 s timer (item and HANDOFF
>   phase from `HR_ITEM` or the branch, `MERGE_HEAD`, ahead/dirty counts, disk above 90%, Postgres
>   down, last gate result); turnless `/gate` (starts TOOL-9's script detached, polls its status
>   file, toasts the result), `/next`, `/validate` (the existing `.sh`/`.ps1` scripts by platform).
>   Tests with `claude plugin test`; `claude plugin validate --strict` in TOOL-9's gate. Open in the
>   plan: whether `agent.spawn` fires for Workflow-tool agents (D3), and whether a skills-directory
>   mod prompts beyond workspace trust. M1 after MOD-91; M2 after TOOL-9.

**TOOL-9** (Tooling findings):

> - [ ] **TOOL-9 - Canonical gate script, committed git hooks, DECISIONS merge rule** (from ANA-29,
>   `docs/ANA-29.md` §4.1, §4.2 D5, D7, D8). `scripts/gate.sh` and `scripts/gate.ps1` run the gate the
>   memory notes describe: `cargo fmt --check`, `cargo clippy --workspace --all-features --
>   -D warnings`, featureless `cargo clippy --workspace -- -D warnings`, tests with `--all-features
>   --no-fail-fast -- --test-threads=1`, a grep for `SIGABRT`/`overflowed its stack`, a check that
>   migration numbers under `crates/htui-store/migrations/` are unique and contiguous, the workflow
>   validator, and `claude plugin validate`/`test` for `.claude/skills/htui-dev` when `claude` ≥
>   2.1.287 is present. It can run detached and writes `target/gate/last.json`. Commit
>   `scripts/githooks/` (pre-commit runs `validate-workflow-docs` with `--scope-paths` on staged
>   workflow docs) and set `core.hooksPath` from `install-workflow-hooks.sh`/`.ps1`. Add
>   `.gitattributes` `DECISIONS.md merge=union`. Not blocked.

**TOOL-10** (Tooling findings):

> - [ ] **TOOL-10 - hr sandbox: stop a run from planting a mod that runs on the host** (from
>   ANA-29, `docs/ANA-29.md` §3.3). `~/.claude` is shared read-write with every run
>   (`docker/hr/compose.hr.yaml:99-114`), and `docs/hr-sandbox.md` already lists "add hooks or
>   settings that execute in your next host Claude session". Mods widen that: a plugin installed,
>   or a `~/.claude/skills/<x>/.claude-plugin` folder written (it loads as `@skills-dir`), inside a
>   sandbox runs **in process** in every later host session, unsandboxed. `~/.claude/dev-mods/` is
>   a lesser risk: a mod there loads only in the session whose ID names its folder, after a
>   hot-reload approval and in a trusted workspace (`mods/create`), so it reaches the host only if
>   that host session is resumed; verify this before deciding whether to mount it read-only. Mount
>   `~/.claude/plugins` (except `plugins/data` and `plugins/store`) and `~/.claude/skills`
>   read-only, or masked, in runs; check that plugin auto-update and `$.store` degrade quietly; and
>   extend the `docs/hr-sandbox.md` table. Not blocked.

**CLEAN-12** (Deferred backlog):

> - [ ] **CLEAN-12 - `.claude/` hygiene found by ANA-29** (from ANA-29, `docs/ANA-29.md` §4.1).
>   Remove the committed `.claude/.headroom_wrap_settings.lock` and ignore it. Decide whether
>   `.claude/skills/generated/` is committed or tool-owned: it is committed, yet `scripts/hr:692`
>   lists it in `TOOL_EXCLUDES` as untracked. Correct `.claude/skills/handoff-run/references/lifecycle.md:98-99` and
>   `install-workflow-hooks.sh:22-23` to what this repo has once TOOL-9 lands (the commit hook via
>   `core.hooksPath`, the `--no-verify` block from TOOL-8). No behaviour change. The lock file and
>   the generated-skills decision are not blocked; the lifecycle text and
>   `install-workflow-hooks.sh` corrections are blocked on TOOL-9, and the `--no-verify` wording on
>   TOOL-8 M1.

**Amendment to MOD-16** (appended to its entry):

> **ANA-29 note (2026-10-08, `docs/ANA-29.md` §8):** once they land, run on Windows MOD-91's
> isolation flags against a subscription login, MOD-92's `--json` verbs (keyring DSN, exit codes),
> and MOD-93's `htui claude emit` with a `claude plugin marketplace add` of the emitted directory,
> with the mod reaching `htui.exe` through `$.process.run`. Mods themselves are documented with
> PowerShell setup and `;` path lists, but native Windows behaviour is unconfirmed.

No other open item owns this work. MOD-47 (the remote human gateway) is not touched: MOD-94 is a
local CLI on the same store, not a relay.

## 9. Rejected ideas

| Idea | Reason |
|---|---|
| One plugin for contributors and consumers | Different audiences, update paths and trust. "Host separate marketplaces for separate audiences" (`plugins/host-marketplace`) |
| Moving workflow skills and agents into a plugin | Namespacing renames them and breaks references. Plain `.claude/` reaches cloud sessions. htui's copies lead the synced surface (`ecosystem-survey.md` "htui revisit"). Rules cannot move at all |
| Installing `htui-dev` into the shared `~/.claude/plugins` | Version-cached, bump-gated, and shared with every sandbox (§3.3) |
| A project `directory` marketplace for `htui-dev` | A session started in a git worktree loads the main checkout's copy (§3.2). Subagent worktrees see the session's copy under either route (§5.2) |
| A consumer MCP server in v1 | Reads need no typed tool surface. A CLI is cheaper in context, works for any agent, and keeps one contract. Revisit when a non-Claude agent needs typed tools |
| Bundling `htui mcp` in a plugin | Token-bound to a session htui launched (§4.3) |
| A mod bridge to htui's socket or an HTTP listener in htui | Protocol mismatch, and a new listener drifts towards a daemon (`R-NF-2`) |
| A hosted git marketplace or a claude.ai listing in v1 | Version drift from the binary, auto-update off by default for the git marketplace, and a paid plan and portal submission for the directory (§5.4) |
| A `command` plugin source | An accept-command prompt, link mode refused on Windows with a 256 MiB copy limit, and a command run every session. U4's relative-path plugin in a local directory marketplace tracks the binary without any of these (§5.4) |
| An out-of-session `command_run` client (`htui queue-run`) | The queue is leased per run and step (`docs/htui-mcp.md`, "When `command_run` is offered"). A caller with no run has no lease, fence or box. That is a new concept, not a CLI verb. Revisit with ANA-28 or MOD-47 |
| Starting or queueing a run from Claude Code | Status moves are the orchestrator's (`R-ENT-8`). A queue verb is a person's action, but no consumer has asked. Revisit after MOD-93 |
| Skill and persona export to `SKILL.md` | Import exists (`R-SKL-4`); export has no requirement and no user |
| Any `$.model` call to classify, route or answer | `R-ID-6` |
| A mod that holds the DSN or writes SQL | `R-STO-1`, `R-ID-7` |
| A mod that auto-approves htui permission prompts | ANA-27 T2: only a person, a configured rule or a cancellation answers |
| htui writing hooks or statusLine into `~/.claude` | ANA-27 §5.2. `emit` writes only its own directory |
| A repo `tool.check` mod that approves what the Gortex hook blocks | It overrides the user's chosen policy. Fix it in Gortex config at user level |
| A sandbox push guard | The sandbox remote has push disabled already |
| Guards that rewrite tool input | Denied in auto mode (§2.3) |
| A UI inside htui-driven sessions | `-p` and ACP draw nothing (§2.4) |

## 10. Requirement changes

Proposed for maintainer approval. Not applied by ANA-29.

**R-ID-5, clarified** (for MOD-91). Add one sentence:

> This includes the user's own harness customizations: in a session `htui` launches, the agent's
> mods, settings hooks and skills are off unless the agent's registry row opts in.

Plugin agents and plugin MCP servers are left out of the sentence on purpose: the levers MOD-91
picks leave them loaded (§2.6, §5.5). Widen it only if MOD-91's probe finds a lever that removes
them.

No other change. MOD-92 and MOD-93 are other presentations of `R-TUI-11`'s list and need no new
requirement. MOD-94 may need a sentence in `R-TUI-4` or `R-TUI-6` if the maintainer wants answers
outside the TUI written down (§12 Q2).

## 11. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| Mods are unsandboxed: a bad `htui-dev` or `htui` mod acts as the user | Low | High | Reach limited to processes (§6.3); `claude plugin validate` `calls:` reviewed in the gate; `htui-dev` changes go through git review |
| A sandbox plants a mod that runs on the host | Medium | High | TOOL-10 |
| User mods inside htui-driven sessions approve calls behind the relay | Low to medium today: latent, since no user mod is installed (§4.1); rises with any mod install, or with `htui-dev` under dogfooding | High | MOD-91, first on impact |
| MOD-91's flag breaks auth or drops htui's MCP server, as `--bare` did | Medium | High | Probe first, like D92; per-row opt-out; `--safe-mode` kept as a fallback only if it keeps `--mcp-config` |
| `sec-default` absent on personal machines, so a user mod can lift a `deny` rule | Medium | Medium | MOD-91 removes user mods from htui sessions. `htui-dev` never approves, it only denies |
| The mods API changes between releases | High | Medium | The plugin works without its mod; `validate` and `test` in the gate; floor checked at `emit`; while developing, Claude Code rewrites the types on each `--plugin-dir` load (not shown for installed plugins, §2.7) |
| CLI below 2.1.287 | Medium | Low | The skill and CLI still work; `emit` omits the mod; `htui-dev` is dev-only |
| Windows: mods inferred but unconfirmed; no platform field in `$` | Medium | Medium | Scripts keep `.ps1` twins; the mod picks by environment; MOD-16 note |
| Mods are Claude Code only, while htui drives `claude`, `claude-cli` and `agy` | Certain | Low | The base layer (CLI JSON, skill text) is agent-neutral. The mod is an extra view, never the only path to a capability |
| A hung guard unloads the mod; three crashes unload every user mod | Low | Medium | No blocking work in hooks; long work detached; fail open with a status warning |
| Polling the store from many sessions | Low | Low | `poll_seconds` setting, 30 s default; one query per poll |
| A dev guard blocks a legitimate command | Medium | Low | Deny text names the fix; guards are narrow regexes with tests |

## 12. Open questions for the maintainer

1. **Isolation default (MOD-91).** Should htui turn the user's mods, settings hooks and skills off
   by default in every session it launches, chat included, with a per-row opt-in? This analysis
   says yes, because `R-ID-5` already says so for skills. Should the managed repo's `CLAUDE.md` also stay out? That
   needs `--safe-mode` or `--bare`, and `--safe-mode` is unproven against `--mcp-config`.
2. **Answers outside the TUI (MOD-94).** May a person answer a relayed permission from a CLI verb or
   a Claude Code button on the same box? ANA-27 put a *remote* human gateway in MOD-47's territory.
   A local CLI on the same store is not that, but it widens who answers.
3. **Is a consumer plugin wanted before a second user exists (MOD-93)?** MOD-92 is useful on its
   own, for scripts and other agents. MOD-93 could wait for a trigger.
4. **Staffing as a hard rule (TOOL-8).** Should the mod deny a Fable spawn outright, or warn?
   **Answered 2026-10-08: deny.**
5. **Read-only sandboxes for plugin paths (TOOL-10).** That stops installing plugins from inside a
   run. Acceptable?
6. **Unverified facts that change the plan if wrong:** whether `$.mcp.call` prompts; whether
   `agent.spawn` fires for Workflow-tool agents; whether a project skills-directory mod loads with
   trust alone, and in cloud sessions; whether `claude-agent-acp` loads user plugins; native
   Windows behaviour of mods.

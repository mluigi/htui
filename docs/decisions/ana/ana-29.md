# ANA-29 - Claude Code mods: developing htui, and using htui in other projects (concluded, 2026-10-08)

Opened as a question about building Claude Code "mods" in two directions: for htui's own development
(what to keep in `.claude/`, what to package as a plugin) and for projects that use htui from Claude Code.
At routing (2026-10-08) the maintainer clarified that "mods" means the Claude Code **mods feature**
(https://code.claude.com/docs/en/plugins/mods/overview): JS/TS event handlers that run inside Claude Code,
ship in a plugin, and can guard tool calls, draw panes and bands, and add commands that need no model turn
(CLI 2.1.287+). Plugins stay in scope as the vehicle ("include the plugins too"). Addresses `R-ID-2`..`R-ID-7`,
`R-STO-1`, `R-AGT-1`, `R-AGT-4`, `R-ENT-8`, `R-HIS-1`, `R-MCP-1`, `R-MCP-3`, `R-MCP-4`, `R-TUI-4`, `R-TUI-6`,
`R-TUI-11`, `R-NF-1`, `R-NF-2`. Analysis: `docs/ANA-29.md`.

**Finding that comes first (`docs/ANA-29.md` §4.4).** htui starts `claude -p` without `--bare` (D92) and with
no other isolation flag, so every session it drives loads the user's plugins, skills, settings hooks and mods.
No user mod is installed today, but any later one could approve a tool call before htui's permission relay
sees it, and `R-ID-5`'s "system skills disabled" is not enforced on the `claude-cli` path.

**Verdict (`docs/ANA-29.md` §6).**
- **MOD-91** turns the user's mods, settings hooks and skills off in htui-driven sessions, after a live probe
  of `--settings '{"disableAllHooks":true}'` and `--disable-slash-commands` against subscription auth and
  htui's own `--mcp-config`.
- **Dev side.** `.claude/` stays as it is (moving skills and agents into a plugin renames them and gains
  nothing; rules cannot move). One new committed `htui-dev` mod at `.claude/skills/htui-dev/` (a
  skills-directory plugin, loaded in place, no install) holds guards for the mistakes now kept only in memory
  notes, a status line and turnless `/gate`, `/next`, `/validate` (**TOOL-8**). Logic stays in `scripts/` with
  `.ps1` twins, starting with a canonical gate script, committed git hooks and a `DECISIONS.md` union merge
  (**TOOL-9**).
- **Consumer side.** A plugin cannot reuse htui's MCP server, whose token is bound to a session htui
  launched. The base is a read-only `--json` CLI any agent can call (**MOD-92**); on top, a thin `htui`
  plugin (one skill, one mod: waiting-on-you band and pane, `/htui` commands) that the binary emits into a
  local directory marketplace, so plugin and binary never drift (**MOD-93**). Answering relayed permissions
  from outside the TUI is **MOD-94**, behind a maintainer decision (§12 Q2).
- **Rules for every htui mod** (§6.3): no `$.model` in a decision path (`R-ID-6`), no DSN or SQL (`R-STO-1`),
  deny rather than rewrite, never answer a prompt for the user, reach limited to running processes.
- **Rejected** (§9): one plugin for both audiences, a consumer MCP server in v1, a bridge to htui's socket, a
  hosted git marketplace or claude.ai listing in v1, an out-of-session `command_run` client, workflow skills
  moved into a plugin.

**Fact-check.** Five parallel readers (mods mechanics, plugin system, dev side, consumer side, prior art)
fed one author; two read-only verifiers (Claude Code facts against the docs and the 2.1.294 CLI; repo facts
against the tree) raised 13 findings, all confirmed and applied before the verdict went to the maintainer.
They narrowed the hazard to what loads by isolation mode (`worktree`/`copy` sessions start outside the repo),
corrected "only a mod can tell agents apart" (settings hooks get `agent_id`/`agent_type`), and fixed the
`zeta` guard's scope and the clippy commands.

**Decisions (maintainer, 2026-10-08).** File all proposed items. The `htui-dev` mod **denies** a Fable spawn
(not a warning). The `R-ID-5` clarification in `docs/ANA-29.md` §10 stays a proposal; §12 Q1, Q2, Q3 and Q5
remain open on their items.

**Spawned.** MOD-91 (session isolation from user mods), MOD-92 (`--json` CLI), MOD-93 (consumer plugin,
blocked on MOD-91 and MOD-92), MOD-94 (permission answers outside the TUI, blocked on a maintainer decision
and MOD-92), TOOL-8 (`htui-dev` mod), TOOL-9 (gate script, git hooks, merge rule), TOOL-10 (hr sandbox: no
planted host mods), CLEAN-12 (`.claude/` hygiene). ANA-29 note appended to MOD-16 (Windows checks).

Commits: analysis and close-out in the commit that added this file.

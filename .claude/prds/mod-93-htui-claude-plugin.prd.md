# MOD-93 - `htui` Claude Code plugin: one skill, one mod, emitted from the binary

Origin: HANDOFF MOD-93 (from ANA-29, `docs/ANA-29.md` §6). Requirements: `R-ID-4`, `R-ID-6`,
`R-STO-1`, `R-TUI-11`, `R-NF-1`. Routed PRD by maintainer accept (2026-10-13). Ultracode
accepted for the implement phase.

## Problem
A person who runs htui and also works in plain `claude` in another project cannot see that htui
is waiting on them (a permission, a question, a finished run) without switching to the TUI. The
`claude` session there does not know htui exists, so it does inline work that should be handed
to htui. The cost is missed waiting items and duplicated work outside htui's queue and history.

## Evidence
- ANA-29 §4.3 confirms the gap: htui's MCP server is token-bound to one session htui launched,
  so a plugin cannot reach it. Until MOD-92 the CLI had no machine-readable output.
- MOD-92 (done) now prints `htui status`, `htui waiting` and `htui --search-items --json` as
  versioned JSON (`docs/htui-cli.md`), but nothing in Claude Code calls them.
- Demand: **Assumption — needs validation by the maintainer's own use beside plain `claude`.**
  ANA-29 §12 Q3 noted there is no second user yet. The maintainer chose to build it anyway by
  accepting MOD-93's run (2026-10-13).

## Users
- **Primary**: a person running htui on this box who is working in plain `claude` in another
  project, and wants htui's waiting-on-you list and a way to hand work to htui from there.
- **Not for**: htui-driven sessions (MOD-91 isolates them, and the mod stands down on
  `HTUI_SESSION`). Not for remote or cross-box users (MOD-47). Not for answering permissions
  (MOD-94).

## Hypothesis
We believe **a plugin emitted from the htui binary, with a waiting-on-you band, a `/htui` pane
and a skill that teaches Claude to read htui and hand work to it**, will **let people see and
route htui work without leaving `claude`** for **people working in plain `claude` beside htui**.
We'll know we're right when **waiting items get noticed from inside `claude` without switching
to the TUI to check, and the skill hands work off instead of doing it inline**.

## Success Metrics
| Metric | Target | How measured |
|---|---|---|
| Waiting items noticed from inside `claude` | Maintainer stops switching to the TUI to check | Maintainer's own use; TBD — needs validation by use over a few weeks |
| Plugin and binary agree | No silent version skew | The mod prints a skew line whenever `htui --version` differs from the plugin's version |
| Plugin is valid | Passes on every gate run | `claude plugin validate --strict` and `claude plugin test`, when `claude` ≥ 2.1.287 is present |
| Install friction | One `marketplace add` plus one `install`, the first time | `emit` prints both commands; nothing else is needed |

## Scope
**MVP** — two milestones. M1: `htui claude emit` and the skill. M2: the mod.

Constraints that hold across both milestones (from ANA-29 §6.3 and §6.4):
- `emit` writes only the directory it is given or the default directory. It never edits
  `~/.claude` and never writes into a repo (`R-ID-4`). It prints the `claude plugin` commands
  and does not run them.
- The plugin's version is the binary's version. The plugin is built from files embedded in the
  binary.
- The mod never calls a model in a decision path (`R-ID-6`). It never holds a DSN or runs SQL
  (`R-STO-1`). Its reach stops at running processes: no `$.http`, no `$.fs.write`, no
  `$.session.send`.
- The mod fails open and visibly, and never holds a hook past its time budget.
- The skill works on any Claude Code version, without the mod.

**Out of scope**
- Answering relayed permissions (MOD-94) — behind a maintainer decision. The pane gains its
  buttons after MOD-93.
- A consumer MCP server — ANA-29 §5.3 S2, overkill for reads.
- A hosted or git marketplace, or a claude.ai listing — ANA-29 §5.4. The plugin and binary
  would drift.
- A streaming `htui watch` — ANA-29 §5.3 S5. Later, if polling is too slow.
- A status-line variant — the maintainer chose the `AbovePrompt` band (2026-10-13). A status
  line would replace the user's own.
- Writes of any kind from the mod.

## Delivery Milestones
<!-- Status: pending | in-progress | complete -->

| # | Milestone | Outcome | Status | Plan |
|---|---|---|---|---|
| 1 | Emit + skill | `htui claude emit [--dir PATH] [--force]` writes a directory marketplace holding plugin `htui` (the binary's version) with skill `htui`, and prints the `marketplace add` / `install` commands. A `claude` session with it installed knows how to read htui state with MOD-92's commands and when to hand work to htui | in-progress | `.claude/plans/mod-93-htui-claude-plugin.plan.md` |
| 2 | Mod | The plugin also carries the `htui` mod: an `AbovePrompt` band with the waiting-on-you count from `htui waiting --json` on a `$.clock.every` timer, a `/htui` pane listing the rows, turnless `/htui status` / `waiting` / `search <q>`, a version-skew line, and stand-down on `HTUI_SESSION`. `userConfig`: `panel` (`auto\|command\|off`), `poll_seconds` (default 30). `emit` leaves out the mod, and says so, when `claude --version` is below 2.1.287. The gate runs `validate --strict` and `plugin test` | in-progress | `.claude/plans/mod-93-htui-claude-plugin.plan.md` |

## Decisions taken at framing (maintainer, 2026-10-13)
- **Surface**: the `AbovePrompt` band, not a status line. `panel` controls the band and pane
  (`auto` shows the band, `command` shows only the pane on `/htui`, `off` shows neither).
- **Default directory**: under htui's data directory (for example `<data>/claude-plugin/`).
  `--dir PATH` overrides it.
- **Re-run**: `emit` refuses a non-empty target unless `--force` is passed. The refusal names
  the flag.
- **Windows**: native mod behaviour is unconfirmed. This is a note for the host and MOD-16's
  Windows check, not a blocker.
- **ANA-29 §12 Q3** (is a consumer plugin wanted before a second user exists?): answered yes, by
  running MOD-93.

## Open Questions
- [ ] Does `--force` replace the whole target directory, or only the files `emit` owns?
  Decide in `/plan`. Replacing the whole directory risks deleting a user's file under `--dir`.
- [ ] Probing `claude --version` at emit time: what does `emit` do when `claude` is not on
  `PATH`? Lean: emit the mod, and say the version could not be checked. TBD in `/plan`.
- [ ] Exact mods API surface on 2.1.296: band registration, `$.clock.every`, `$.process.run`,
  `userConfig` shape, and `claude plugin test` fixtures. The plan verifies these against the
  installed CLI and its docs, not from recall.

## Risks
| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| The mods API changes between releases | High | Medium | The plugin works without its mod. `validate` and `test` run in the gate. The floor is checked at `emit` |
| The mod loads inside an htui-driven session | Low | High | MOD-91 isolation, plus the mod stands down on `HTUI_SESSION` |
| The plugin and binary drift after an upgrade | Medium | Low | The skew line, and re-running `emit --force` |
| Polling load from many sessions | Low | Low | `poll_seconds` (30 s default), one `htui waiting --json` per poll |
| An unsandboxed mod acts as the user | Low | High | Reach limited to processes. The `validate` `calls:` line is reviewed in the gate |
| Native Windows behaviour of mods | Medium | Medium | `htui.exe` reached through `$.process.run`. Host and MOD-16 check |
| `emit --force` deletes user content under a custom `--dir` | Low | Medium | Open question above; the plan decides the safe shape |

---
*Status: DRAFT — requirements only. Implementation planning pending via /plan.*

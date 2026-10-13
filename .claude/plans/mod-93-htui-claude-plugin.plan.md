# Plan: MOD-93 - `htui` Claude Code plugin, emitted from the binary

**Source PRD**: `.claude/prds/mod-93-htui-claude-plugin.prd.md`
**Selected Milestone**: 1 (emit + skill) and 2 (mod), in one plan. The emit verb and the mod's
omission path share one Rust file, and splitting them would serialise the fan-out for no gain.
The maintainer can split them at CONFIRM.
**Complexity**: Medium (about 16 files; one new Rust module, one new plugin tree, one doc).
**Routing**: PRD path, accepted by the maintainer (2026-10-13). Ultracode is accepted for the
implement phase.

## Summary

`htui claude emit [--dir PATH] [--force]` writes a directory marketplace from files embedded in
the binary, and prints the two `claude plugin` commands. The marketplace holds plugin `htui`,
stamped with the binary's version. The plugin has a skill `htui`, which teaches Claude Code to
read htui through MOD-92's JSON commands and to hand work to htui. It also has a mod: an
`AbovePrompt` band with the waiting-on-you count, a `/htui` pane, turnless `/htui status|waiting|search`,
a version-skew line, and stand-down on `HTUI_SESSION`. The plugin source lives in
`crates/htui/claude-plugin/`, never under `.claude/`, so this repo's own sessions never load it.

## Decisions (this plan)

| # | Decision | Why |
|---|---|---|
| D1 | The plugin source tree is `crates/htui/claude-plugin/` (`.claude-plugin/marketplace.json`, `htui/…`). `claude_emit.rs` embeds every file with `include_str!` in one explicit table | Explicit beats a build-script walk. A file missing from the table fails a unit test (D10), not the build. |
| D2 | The marketplace and the plugin are both named `htui`. The install command is `claude plugin install htui@htui` | Probed: both validate under `--strict` and install as `htui@htui`. |
| D3 | The default directory is `dirs::data_local_dir()/htui/claude-plugin` (`~/.local/share/htui/claude-plugin` on Linux). `--dir PATH` overrides it. No data dir means a refusal (exit 2) | Matches the sibling `htui/agents` install root (`htui-agent/src/probe.rs:205`). |
| D4 | A target that exists and is non-empty is refused unless `--force`. Even with `--force`, `emit` refuses a directory it did not write: one holding any entry but `.claude-plugin` and `htui`, or whose `.claude-plugin/marketplace.json` is not a marketplace named `htui`. `--force` stages into a sibling `<dir>.emit-<pid>`, renames the old directory aside, renames the staging directory into place, then removes the old one | Settles PRD open question 1: `--force` can never delete a user's files under a custom `--dir`. |
| D5 | Version stamping: `plugin.json`'s `version` and `hooks/version.ts` (`export const VERSION = '<v>'`) are written with `env!("CARGO_PKG_VERSION")`. The source tree carries `0.0.0-source` | The plugin version is the binary's version (PRD). The source tree stays edit-free across version bumps. |
| D6 | Floor: `emit` probes `claude --version` with `htui_agent::probe::capture_version`, pattern `^(\d+\.\d+\.\d+)`, compared with `below_min(v, "2.1.287")`. If it is below the floor, `emit` writes the skill only: no `hooks/`, no `types/`, and `plugin.json` without `types` and `userConfig`. It says so in one line. If `claude` is not on `PATH` or prints no version, `emit` writes the mod and says the version was not checked | Settles PRD open question 2. Reuses the bounded, killable probe child rather than adding a new process path. |
| D7 | `emit` never writes outside the target directory and its staging siblings, never runs `claude plugin …`, and never touches `~/.claude` (`R-ID-4`) | ANA-29 §5.4 U4 and §6.4. |
| D8 | `EmitExit { Refused(String) = 2, Failed(String) = 1 }`, mapped in `main.rs::exit_code`. No `EmitExit` goes to Sentry: its sentences carry home paths, the same as `KeysError` | Mirrors `StateExit` and `KeysError`. |
| D9 | `*.test.ts` files are not emitted. The gate runs `claude plugin test` on the source tree and `claude plugin validate --strict` on emitted copies (with the mod and skill-only) | Users don't need the kit tests. Validating what users actually get catches stamping errors. |
| D10 | Gate test `crates/htui/tests/claude_plugin.rs`, **not** behind `testkit`. When `claude` ≥ 2.1.287 is on `PATH`, it emits into a temp dir and runs `validate --strict` (marketplace and plugin) and `plugin test` (source tree), with `CLAUDE_CONFIG_DIR` set to a temp dir. Otherwise it prints one skip line and passes | ANA-29 §6.3 "gate the mod". No `scripts/gate.sh` exists yet (TOOL-9). The `CLAUDE_CONFIG_DIR` setting keeps it out of the shared `~/.claude`. |
| D11 | The mod's scope is no flag: the first workspace, as the TUI's top bar shows it. A `--cwd` scope is out of v1 | `--cwd` refuses `cwd_unmatched` in any project htui does not manage, which is most of them for this audience. |
| D12 | `panel`: `auto` shows the band (when the count is above 0, or there is a skew or error line) and `/htui` opens the pane. `command`: no band, and `/htui` opens the pane. `off`: no band and no pane, so `/htui` prints the waiting rows as text. `poll_seconds` defaults to 30, minimum 5. Polling runs only while the band is on or the pane is open | PRD framing decision. Keeps the per-poll cost (one process start plus one query) off when nothing shows the result. |
| D13 | Every `$.process.run` sets `timeoutMs`: 10 s for `waiting`, `status` and `--version`, 60 s for search (it may load the embedding model). A rejection (not on PATH, timeout) becomes a visible line, `htui: not found on PATH` or `htui: timed out`, never a throw | "Fail open, visibly" (ANA-29 §6.3). |
| D14 | Skew: at `session.start` the mod runs `htui --version` once and parses `htui <v>`. If it differs from `VERSION`, the band and `/htui status` show `htui <bin> ≠ plugin <VERSION>: run htui claude emit --force, then /reload-plugins` | PRD item (4). Probed: a directory-marketplace plugin is read from the folder, so a re-emit plus reload is enough. |
| D15 | Stand-down: if `$.env.get('HTUI_SESSION')` is set, `session.start` registers nothing and starts no timer, and every other hook returns `next(e)` | PRD item (5). MOD-91 sets `HTUI_SESSION` (`htui-agent/src/launch.rs:607`). |

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Subcommand | `crates/htui/src/cli.rs:120-144` (`Command`), `:173-184` (`StateArgs`) | A doc comment on every variant and flag, says what is read or written; `clap::Args` structs; parse tests in the same file's `mod tests` |
| Dispatch | `crates/htui/src/lib.rs:117-128` | `if let Some(cli::Command::X(args)) = args.command.clone() { return x::run(args).await.map_err(anyhow::Error::from); }`, placed after `init_tracing`, before the keyring flags |
| Exit type | `crates/htui/src/state_cmd.rs:50-83` (`StateExit`) | An enum of sentence-carrying arms, `const fn code()`, `Display` = the sentence, `Error` impl |
| Exit mapping | `crates/htui/src/main.rs:103-148` (`exit_code`, `reports_to_sentry`) | One more `.or_else(downcast_ref::<EmitExit>)`, and one more `is_none()` clause in `reports_to_sentry` |
| Version probe | `crates/htui-agent/src/probe.rs:1118` (`capture_version`), `:1017` (`below_min`) | A bounded child, killed on timeout; semver compare where unparsable means not below |
| Data dir | `crates/htui-agent/src/probe.rs:205` (`default_install_root`) | `dirs::data_local_dir().map(\|d\| d.join("htui").join(...))` |
| Tests | `crates/htui/src/state_cmd.rs` `mod tests`, `crates/htui/src/cli.rs` `mod tests` | Unit tests beside the code, named as sentences (`a_foreign_directory_is_refused_even_with_force`) |
| Mod shape | `plugin-authoring` skill examples `band.tsx`, `pane.tsx` (CLI 2.1.296), and the probe below | `atom`/`read`/`update` over `$.state`, with a `types/index.d.ts` contract; `$.command.register` in `session.start`; `command.run` answering `{ text }` |
| Mod tests | probe `hooks/htui.test.ts` (verified, 3/3 pass) | `on('process.run', …)` answers `{ value: { exitCode, stdout, stderr } }`; `on('session.start', ($, e) => e)` beneath; `mock.clock(on)` + `advance`; `mock.env(on, {...})`; `$.ui.mount({ plugin, surface, component: 'AbovePrompt', props })` looped over `['terminal', 'desktop']` |

## Files to Change

| File | Action | Task |
|---|---|---|
| `crates/htui/claude-plugin/.claude-plugin/marketplace.json` | CREATE | T1 |
| `crates/htui/claude-plugin/htui/.claude-plugin/plugin.json` | CREATE (version `0.0.0-source`, `types`, `userConfig` `panel`/`poll_seconds`) | T1 |
| `crates/htui/claude-plugin/htui/skills/htui/SKILL.md` | CREATE | T1 |
| `crates/htui/claude-plugin/htui/hooks/hooks.json` | CREATE | T2 |
| `crates/htui/claude-plugin/htui/hooks/register.tsx` | CREATE | T2 |
| `crates/htui/claude-plugin/htui/hooks/version.ts` | CREATE (`0.0.0-source`) | T2 |
| `crates/htui/claude-plugin/htui/hooks/*.test.ts` | CREATE | T2 |
| `crates/htui/claude-plugin/htui/types/index.d.ts` | CREATE | T2 |
| `crates/htui/src/claude_emit.rs` | CREATE | T3 |
| `crates/htui/src/cli.rs` | UPDATE (`Command::Claude(ClaudeArgs)`, `ClaudeCommand::Emit(EmitArgs)`, parse tests) | T3 |
| `crates/htui/src/lib.rs` | UPDATE (`pub mod claude_emit`, dispatch) | T3 |
| `crates/htui/src/main.rs` | UPDATE (`exit_code`, `reports_to_sentry`, their tests) | T3 |
| `crates/htui/Cargo.toml`, `Cargo.lock` | UPDATE (`dirs` from the workspace: already in the lock through htui-store) | T3 |
| `crates/htui/tests/claude_plugin.rs` | CREATE (gate, D10) | T3 |
| `docs/htui-claude-plugin.md` | CREATE | T4 |
| `docs/htui-cli.md` | UPDATE (one "see also" line under the intro) | T4 |
| `README.md` | UPDATE (a short "Use htui from Claude Code" section) | T4 |

File sets: T1 ∩ T2 = ∅, and T3 ∩ T4 = ∅ (checked against the table above). T3 depends on T1 and
T2 (its `include_str!` table names their files). T4 depends on the decisions only.

**Waves**: W1 = T1 ∥ T2. W2 = T3 ∥ T4. Then the review gate.

## Tasks

### Task 1: plugin manifests and skill (M1)
- **Action**:
  - `marketplace.json`: `{ name: "htui", owner: { name: "htui" }, description, plugins: [{ name: "htui", source: "./htui", description }] }`.
  - `plugin.json`: `name`, `version: "0.0.0-source"`, `description`, `author`, `types: "./types/index.d.ts"`, and `userConfig`:
    - `panel`: `{ type: "string", title, description, default: "auto", options: ["auto","command","off"] }`
    - `poll_seconds`: `{ type: "number", title, description, default: 30 }`
  - `SKILL.md`, frontmatter `name: htui`, with a `description` that triggers on htui state and handing work over. The body:
    - when to use it;
    - the three commands with `--json`;
    - the envelope (`version`, `kind`, `store`);
    - exit codes 0/3/2/1, and that empty stdout is a failure;
    - "ignore unknown fields";
    - how to read `waiting` rows;
    - **hand-off rules**: when a request matches an open htui item or belongs in htui's queue, say so and point the person at the TUI. Never work it inline, and never try to write htui's store; there is no write command.
    - Link `docs/htui-cli.md` by its repository URL, not by path: the skill runs in other repos.
- **Mirror**: the probe's manifests (verified); `docs/htui-cli.md` for every schema fact. Restate nothing the doc doesn't say.
- **Validate**: `claude plugin validate --strict crates/htui/claude-plugin` and `… /htui`. With T2 absent, `types` points at a missing file, so validate after W1 merges.

### Task 2: the mod (M2)
- **Action**:
  - `hooks.json`: `{ "modules": ["./register.tsx"] }`.
  - `types/index.d.ts`: the `PluginState` contract under `htui`: the waiting snapshot (`rows`, `at`, `error`), the skew line, `isPaneOpen`.
  - `version.ts`.
  - `register.tsx`, per D11-D15:
    - `session.start`: stand down on `HTUI_SESSION`; register `/htui` with `argumentHint: "[status|waiting|search <query>]"`; run the version check; start `$.clock.every(poll_seconds*1000)` polling `htui waiting --json` while the band or pane needs it.
    - `command.run {command:'htui'}`: route on `e.args`. Empty opens the pane, or prints rows when `panel=off`. `status`, `waiting` and `search <q>` (`htui --search-items <q> --json`) answer `{ text }`, formatted from the documents. A non-zero exit shows `error.message`; empty stdout shows "htui printed nothing (exit N)".
    - `ui.render AbovePrompt`: `next(e)` when `panel != auto`, or there is nothing to show, or `e.props.hasSurvey`. Otherwise a `Box` row: `htui: N waiting on you`, plus the skew or error line.
    - `ui.render Pane {requestId:'htui'}`: rows of reason, item key, step, text and age, sized to `e.props.bodyColumns`.
  - Reach is `$.process`, `$.clock`, `$.env.get`, `$.command`, `$.state`, `$.ui` only. No `$.model`, `$.http`, `$.fs`, `$.session.send` or `$.session.append`.
  - `*.test.ts` covering:
    - the band drawn on terminal and desktop with N>0, and nothing at 0;
    - `panel=command`/`off` (via `test(name, { options }, body)`);
    - stand-down;
    - the skew line;
    - offline exit 3, refused exit 2, and a `process.run` rejection, each producing a visible line and no throw;
    - each `/htui` sub-command's text;
    - the `search` argv carrying the query as one argument.
- **Mirror**: the probe (`/tmp/mod93-probe/htui`, verified), plus the `plugin-authoring` skill's `band.tsx`/`pane.tsx`. The API authority is the `.d.ts` the skill lays down (`grep` it; don't trust recall).
- **Validate**: `claude plugin validate --strict crates/htui/claude-plugin/htui` (its `calls:` line lists only the nouns above) and `claude plugin test crates/htui/claude-plugin/htui`.

### Task 3: `htui claude emit` and the gate test (M1 + M2)
- **Action**:
  - `cli.rs`: `Command::Claude(ClaudeArgs)` with a nested `#[command(subcommand)] ClaudeCommand::Emit(EmitArgs { dir: Option<PathBuf>, force: bool })`, plus parse tests: flags, `--force` alone, no TUI flags beside it, `--log` after it.
  - `claude_emit.rs`:
    - the embedded file table (D1); the target choice (D3); the refuse/force rules and staging (D4); stamping (D5); the floor (D6); `EmitExit` (D8);
    - a `pub fn write_to(dir, &Options) -> Result<Report, EmitExit>` seam the gate test calls;
    - `run(args)` printing the report: the target, the version, mod included / omitted / unchecked, the two commands, and the re-run hint.
  - `lib.rs` dispatch after `init_tracing`. `main.rs` mapping.
  - Unit tests first (TDD):
    - every source file is in the table (walk `CARGO_MANIFEST_DIR/claude-plugin`, minus `*.test.ts`), and the table names no missing file;
    - the stamped version equals `CARGO_PKG_VERSION` in both places;
    - the skill-only variant has no `hooks/`/`types/` and its `plugin.json` has no `types`/`userConfig`;
    - an empty dir works; a non-empty dir is refused without `--force`; `--force` on our own dir replaces it with no stale file left; `--force` on a foreign dir (an extra file, or a foreign `marketplace.json`) is refused and the dir is untouched;
    - no write outside the target and its siblings;
    - exit codes and the Sentry rule.
  - `tests/claude_plugin.rs` (D10).
- **Mirror**: `state_cmd.rs` (exit type, module doc listing the `R-` IDs), `cli.rs` tests, `main.rs` tests (`state_exits_are_3_2_1_and_only_failures_reach_sentry`).
- **Validate**: `cargo test -p htui claude` (unit tests and the gate test); a manual `cargo run -p htui -- claude emit --dir /tmp/x`, then `claude plugin validate --strict /tmp/x`.

### Task 4: docs
- **Action**:
  - `docs/htui-claude-plugin.md`:
    - what the plugin is and who it is for;
    - `htui claude emit`'s flags, default dir, `--force` rules and exit codes;
    - the two install commands, and re-running after an upgrade (`emit --force` + `/reload-plugins`);
    - `panel` and `poll_seconds` (and `/plugin configure htui@htui`);
    - what the band, pane and commands show;
    - stand-down in htui-driven sessions;
    - the floor and the skill-only fallback;
    - what it never does (no `~/.claude` edits, no writes, no model calls, reach limited to running `htui`);
    - Windows: unconfirmed, see MOD-16.
  - A "see also" line in `docs/htui-cli.md`; a README section.
- **Mirror**: `docs/htui-cli.md`'s voice: plain sentences, tables for flags and codes.
- **Validate**: every flag, code and path named in the doc matches T3's code (the reviewer checks).

## Validation

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo clippy --workspace -- -D warnings                      # featureless gate (memory)
cargo test -p htui --lib claude_emit
cargo test -p htui --test claude_plugin                        # runs claude when ≥ 2.1.287 is on PATH
cargo test -p htui cli::tests
claude plugin validate --strict crates/htui/claude-plugin
claude plugin validate --strict crates/htui/claude-plugin/htui
claude plugin test crates/htui/claude-plugin/htui
cargo test --workspace --all-features -- --test-threads=1      # full gate before close-out (memory)
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| The mods API moves after 2.1.296 | High | The floor is checked at `emit`; `validate` and `test` run in the gate; the skill works alone |
| No `tsc` here, so type errors in `register.tsx` slip past | Medium | `validate --strict` reads the module the way the engine does, and `plugin test` runs it. The type-check is the host's (`tsc -p`, when a toolchain exists) |
| `include_str!` table drift | Medium | A unit test walks the source tree against the table |
| `--force` deletes user data | Low | D4's ownership check, plus a test on a foreign directory |
| The gate test spawns `claude`, which touches `~/.claude` | Low | `CLAUDE_CONFIG_DIR` set to a temp dir (probed: install and validate wrote nothing into the plugin folder or the shared config) |
| A hung `htui` blocks the band | Low | `timeoutMs` on every run (D13); polls are timer callbacks, not hooks |

## Verified claims

| Claim | Verdict | Evidence |
|---|---|---|
| Subcommands are a `clap::Subcommand` enum in `cli.rs`, dispatched in `lib.rs::run`, with exit codes in `main.rs::exit_code` | ✔ | `cli.rs:120-144`, `lib.rs:99-128`, `main.rs:103-129` |
| `htui` has a clap `version` attribute (so `htui --version` prints `htui <v>`) | ✔ source / runtime pending | `cli.rs:10` `version`. T3 adds a test on `Args::command().render_version()` |
| `claude` ≥ 2.1.287 is present in the sandbox | ✔ | `claude --version` → `2.1.296 (Claude Code)` |
| A `userConfig` string with `options` and a number field validate under `--strict` | ✔ | probe `plugin.json`: "Validation passed" |
| A directory marketplace with `source: "./htui"`, named `htui`, validates, adds and installs as `htui@htui` | ✔ | probe with a scratch `CLAUDE_CONFIG_DIR`: `Successfully installed plugin: htui@htui`; list says `Read from: /tmp/mod93-probe/htui` |
| An installed directory-marketplace plugin is read from the folder (re-emit + `/reload-plugins`, no reinstall) | ✔ | `claude plugin list` "Read from:" plus plugin-authoring reference.md, "Developing one" |
| `validate`, `test` and `install` write nothing into the plugin folder | ✔ | `find /tmp/mod93-probe -type f` after all three: only the authored files |
| `$.process.run(argv, {timeoutMs})`, `$.clock.every`, `$.env.get(literal)`, `$.command.register({argumentHint})`, `command.run` → `{ text }` exist on 2.1.296 | ✔ | `claude-code.d.ts:3469,3522,3595,1708,1843`; validate's `calls:` line |
| `validate` reports reach (`calls:`, env reads, state reads/writes) | ✔ | probe output lists `$.clock.every, $.command.register, $.env.get, $.process.run, $.state.get, $.state.set, $.ui.resolve` |
| The kit answers `process.run` beneath with `{ value }`, mocks the clock and env, and mounts `AbovePrompt` on terminal and desktop | ✔ | probe `htui.test.ts`: 3 pass, 0 fail |
| `Text` drops `key` in a mounted tree; find by type and text, or key a `Box` | ✔ | the probe's `findAll` dump |
| `HTUI_SESSION` is MOD-91's marker | ✔ | `htui-agent/src/launch.rs:607` `SESSION_VAR` |
| `capture_version` and `below_min` are public and reusable | ✔ | `htui-agent/src/probe.rs:1017,1118`, `pub mod probe` (`lib.rs:124`) |
| `dirs` is a workspace dep and already in the lock; `htui` doesn't list it | ✔ | root `Cargo.toml:91` `dirs = "6"`; `crates/htui/Cargo.toml`; `htui-store/src/identity.rs:52` uses it |
| No gate script exists (TOOL-9 not landed) | ✔ | `ls scripts` → `hr`, `hr-mint`, `scrub-audit.sql` |
| No `tsc`/`node`/`bun` in the sandbox | ✔ | `command -v` → none |
| T1 ∩ T2 = ∅ and T3 ∩ T4 = ∅ | ✔ | Files to Change table |
| A new `tests/*.rs` without `#![cfg(feature = "testkit")]` runs in a plain `cargo test` | ✔ by construction | the existing ones carry that attribute explicitly (`tests/waiting.rs:10`); the new file omits it |

## Acceptance
- [ ] All tasks complete; each commits incrementally
- [ ] The validation block passes (`claude` steps on this sandbox's 2.1.296)
- [ ] Patterns mirrored, not reinvented
- [ ] `rust-reviewer` gate run over the full change set; findings applied or deferred with the maintainer

## Amendments accepted at blueprint (maintainer, 2026-10-13)

`.claude/plans/mod-93-htui-claude-plugin.blueprint.md` A-1 to A-12 supersede the plan where they
differ: the kit tests live in `crates/htui/claude-plugin/htui/tests/` (A-1); the `/htui`
`command.run` hook never reads `next` (A-2); no `isPaneOpen` state, the poll reads `$.ui.panes()`
(A-4); SKILL.md names `docs/htui-cli.md` and the clone URL without a web link (A-7).

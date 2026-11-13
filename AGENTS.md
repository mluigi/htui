# htui — agent instructions

The one instruction file for every coding agent in this repo. Claude Code (v2.1.277 or later)
and Cline both read this `AGENTS.md` directly. Put instructions here, never in a `CLAUDE.md`,
`.claude/CLAUDE.md` or `CLAUDE.local.md`: while one exists, Claude Code reads it *instead of* this
file. The one harmless exception is a `CLAUDE.md` holding only `@AGENTS.md` (what `gortex init`
writes), which loads this file through the import and never twice.

`htui` is a Rust TUI that runs a backlog of work items through coding agents across projects and
boxes. Product contract: `docs/REQUIREMENTS.md` (`R-<AREA>-<N>`). Standing invariants:
`CONCEPTS.md`. Outstanding work: `HANDOFF.md`, read it first; history: `DECISIONS.md`.

## Rules

Path-scoped rules load automatically when you touch the files they govern. Each rule has two
byte-identical copies because each agent reads only its own directory: `.claude/rules/<name>.md`
(Claude Code) and `.clinerules/<name>.md` (Cline).

- `workflow-docs.md` — `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/**`, `docs/ANA-*.md`.
- `concept-docs.md` — `CONCEPTS.md`.

Edit both copies in the same commit. `validate-workflow-docs` reports `rule-copy` when they differ,
and the pre-commit hook blocks a commit that stages one copy out of step.

## Skills and workflow

`.claude/skills/` holds the workflow skills (`handoff-add`, `handoff-run`, `plan`, `plan-prd`,
`graphify`); both agents discover them there. This repo is their source of truth: edit them here.
Overview: `.claude/skills/handoff-docs.md`.

## Verify, commit, push

- Gate: `bash scripts/gate.sh` (Windows: `pwsh -NoProfile -File scripts/gate.ps1`). On Windows
  always run the `.ps1` twin of a script, never the `.sh`.
- Integration tests: one binary per crate (TOOL-15). Each crate's `tests/<name>.rs` is the module
  `tests/it/<name>.rs` of the `it` target, so run one file as
  `cargo test -p <crate> --all-features --test it <name>::`, never `--test <name>`. The filter is a
  libtest **substring** match: `settings::` also runs `box_settings::` and its kin, so read the
  names that run. Snapshots live in `tests/it/snapshots/it__<name>__<test>.snap`. Without
  `--all-features` (or `--features testkit` for `htui`, `test-support` for `htui-core` and
  `htui-agent`) the `it` binary fails on a canary test instead of passing with 0 tests. Under
  `cargo test` a crash in one test (stack overflow, SIGABRT) ends its crate's whole `it` binary;
  under nextest it ends that test. `abort-grep` in the gate names it either way. Two files stay
  outside `it`: `htui-agent/tests/extensibility.rs` (its path is hard-coded in the `htui-dev` mod)
  and `htui/tests/panic_hook.rs` (it installs a process-wide panic hook that, under `cargo test`,
  would see every parallel test's panics). A new test that sets process-wide state (`set_hook`,
  environment, cwd) gets its own top-level `tests/<name>.rs` target, or re-runs itself as a child,
  so it stays correct under both runners.
- Test runners (TOOL-17): the gate's `test` step runs
  `cargo nextest run --workspace --all-features --no-fail-fast --profile gate` (one process per
  test; a test that passes on its retry is named `FLAKY`), then
  `cargo test --workspace --all-features --doc`, since nextest runs no doctests. Both runners work
  for item runs, at default threads:
  `cargo nextest run -p <crate> --all-features -E 'binary(it) and test(/^<module>::/)'` (without
  `binary(it)` the filter also runs the crate's unit tests in a same-named `src/` module) or
  `cargo test -p <crate> --all-features --test it <module>::`. Under `cargo test` a crate's tests
  share one process and run side by side (TOOL-16), so the rules below are written for that case.
- A test of the live Qdrant lives in `qdrant_live`/`qdrant_worker` or its name starts with
  `live_qdrant_`: `.config/nextest.toml` runs those one at a time in the `live-qdrant` group
  (`cargo nextest show-config test-groups --all-features` lists it). It still holds
  `htui_store::testkit::qdrant_turn()` for its whole body, as a keyring test holds `mock_keyring()`:
  the locks serialise it under `cargo test` and cost nothing under nextest. It creates its
  collection with `testkit::qdrant_connect_to` (retries the client's 5 s deadline under load). A
  test that writes a script and executes it writes it with `tests/rust/write_executable.rs`
  (included through `#[path]`), never `fs::write` plus `chmod` (`ETXTBSY`).
  `testkit::fresh_db`/`demo_db` copy a template database, so `migrations_at_connect` is
  `UpToDate`; a test about pending migrations starts from `bare_db`.
- The `#[ignore]`d live agent cases in `htui-agent`'s `agy_live`, `auth_live`, `cli_live` and
  `isolation_live` take their module's `ONE_AT_A_TIME` lock, which serialises them under
  `cargo test`; under nextest the `live-agent` group in `.config/nextest.toml` does, and a new
  `*_live` module with such a lock joins that group's filter.
- Workflow docs alone: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh`.
- Commit hooks live in `scripts/githooks/`; `install-workflow-hooks.sh`/`.ps1` sets
  `core.hooksPath` to them. Escape hatch: `WORKFLOW_SKIP_HOOK=1`, never `--no-verify`.
- **Push policy:** never push unless the maintainer asks for it in this session. `origin` is the
  private forge, `github` the public mirror.

## Gortex: code navigation and edits

A Gortex MCP server indexes this repo (`.gortex.yaml`). Prefer its graph tools over plain file
reads and grep for indexed Rust source. Tools take an `operation`; the agent may prefix the tool
name (`mcp__gortex__search` in Claude Code, `gortex__search` in Cline).

1. Liveness: `workspace(operation:"index")`. If the index is empty,
   `workspace_admin(operation:"index")` on `.` first.
2. Orient: `explore(operation:"task")` for a change or diagnosis, `explore(operation:"localize")`
   to find where something lives. For one known symbol go straight to
   `search(operation:"symbols")` and `read(operation:"source")`.
3. Before editing a file: `read(operation:"editing_context")`.
4. Before changing a symbol: `change(operation:"impact")`; for a signature change also
   `change(operation:"verify")`. Refactors: `change(operation:"edit_plan")`, then
   `edit(operation:"batch")`.
5. After editing: `change(operation:"detect")`, then `change(operation:"tests")` on its symbols.
   The project's real build and tests (the gate) are the verification.

The graph narrows scope; source confirms behaviour. Read the full body with
`read(operation:"source")` before changing or depending on a symbol, and never compress bodies for
behaviour-critical code: migrations, retry, fallback and recovery paths, compatibility shims,
concurrency-sensitive sections and the tests that pin them. `capabilities` gives an operation's
exact fields; `gortex://guide` is the full reference.

The block below is generated by `gortex init`; do not edit inside its markers.

<!-- gortex:communities:start -->
## Community Skills

| Area | Description | Explore |
|------|-------------|---------|
| Src Model 25 Dirs | 1316 symbols | `analyze(operation:"communities", id:"community-111")` |
| Htui Orch Src 6 Dirs | 972 symbols | `analyze(operation:"communities", id:"community-117")` |
| Htui Tests 14 Dirs | 796 symbols | `analyze(operation:"communities", id:"community-233")` |
| Tabs Settings 11 Dirs | 781 symbols | `analyze(operation:"communities", id:"community-257")` |
| Src Isolate 10 Dirs | 659 symbols | `analyze(operation:"communities", id:"community-143")` |
| Src Model 3 Dirs Matches | 636 symbols | `analyze(operation:"communities", id:"community-98")` |
| Htui Store Src 9 Dirs | 578 symbols | `analyze(operation:"communities", id:"community-104")` |
| Htui Store Tests 13 Dirs | 575 symbols | `analyze(operation:"communities", id:"community-218")` |
| Htui Orch Src 3 Dirs Steps Of | 489 symbols | `analyze(operation:"communities", id:"community-115")` |
| Src Model 14 Dirs | 445 symbols | `analyze(operation:"communities", id:"community-244")` |
| Htui Agent Src 4 Dirs Record | 440 symbols | `analyze(operation:"communities", id:"community-25")` |
| Htui Agent Tests 4 Dirs Json | 365 symbols | `analyze(operation:"communities", id:"community-40")` |
| Src Prompt 5 Dirs | 315 symbols | `analyze(operation:"communities", id:"community-77")` |
| Htui Tests 9 Dirs | 284 symbols | `analyze(operation:"communities", id:"community-240")` |
| Htui Agent Src 7 Dirs | 283 symbols | `analyze(operation:"communities", id:"community-9")` |
| Src App 5 Dirs | 250 symbols | `analyze(operation:"communities", id:"community-236")` |
| Htui Orch Src 12 Dirs | 206 symbols | `analyze(operation:"communities", id:"community-11")` |
| Htui Agent Session Main | 206 symbols | `analyze(operation:"communities", id:"community-4")` |
| Htui Orch New | 202 symbols | `analyze(operation:"communities", id:"community-156")` |
| Src Install 4 Dirs | 188 symbols | `analyze(operation:"communities", id:"community-54")` |

<!-- gortex:communities:end -->

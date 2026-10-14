# Plan: TOOL-10 - hr sandbox: stop a run from planting a mod that runs on the host

**Source**: `HANDOFF.md` TOOL-10 (from ANA-29 §3.3). Routed as plan by the maintainer (1 criterion fired: C3).
**Selected Milestone**: single milestone
**Complexity**: Small

## Summary

`~/.claude` is bind-mounted read-write into every run, so a hostile run can install a plugin, or write
`~/.claude/skills/<x>/.claude-plugin`, and the mod in it runs in process in the next host session.
Mount `~/.claude/plugins` and `~/.claude/skills` read-only in runs, keep `plugins/data` and
`plugins/store` read-write, and mask `~/.claude/dev-mods` with a per-container tmpfs. Mirror the
existing `~/.claude` mask pattern in `compose.hr.yaml`, `scripts/hr` and `tests/hr`, then extend
`docs/hr-sandbox.md`. This closes the **mod-installing** routes only: `settings.json` stays read-write
(see Residual and D1).

## Requirements restatement

1. A run cannot add or change a plugin, a marketplace, or a skills-directory plugin that the host loads.
2. `plugins/data` (`${CLAUDE_PLUGIN_DATA}`) and `plugins/store` (`$.store` files) stay writable: plugins keep state.
3. `dev-mods`: decide read-only vs masked (the item says verify first - done below).
4. Plugin auto-update and `$.store` degrade quietly (checked by probe T0, not assumed).
5. `docs/hr-sandbox.md` table and threat model say what changed and what did not.

## Verified claims (plan fact-check, step 3.5)

| Claim | Verdict | Evidence |
|---|---|---|
| `~/.claude` is one rw bind; `plugins`, `skills`, `dev-mods` are not masked | TRUE | `docker/hr/compose.hr.yaml` volumes: only the tmpfs list + 4 log files + the run's project are layered over the bind |
| Layering a child mount over a parent already works | TRUE | `projects` tmpfs, then the run's project bound back rw over it (`compose.hr.yaml`, test 42 asserts the order) |
| `plugins/data` and `plugins/store` exist on the host as directories | TRUE | `ls ~/.claude/plugins`: `data/` (16 plugin dirs), `store/` (one `cc-plugin-diff_builtin-*.json`) |
| Plugin data dirs hold no executable code today | TRUE (today) | `ecc-ecc`, `remember-*`, `security-guidance-*` data dirs are empty. A plugin could put a venv there later: residual |
| A mod Claude writes in a session lives in `~/.claude/dev-mods/<session id>/` and loads only in that session, after hot-reload approval, in a trusted workspace | TRUE | https://code.claude.com/docs/en/plugins/mods/create ("loads only in the session that made it"; "Your answer lasts for the session, including after you resume it") |
| ...so it reaches the host only if the host resumes that run session | TRUE but reachable | The clone sits at the host repo path, so the run's own `projects/<name>` dir (transcripts) is the one shared rw bind: the host can `--resume` a run's session ID, and the stored approval then lasts |
| `skills/` has entries a run may write | TRUE | `skills/learned` (empty), `skills/graphify`, 20 `gortex-*` dirs, `find-skills -> ../../.agents/skills` (dangling in a run already: `~/.agents` is not mounted) |
| `~/.claude/settings.json` remains rw after this change | TRUE (by design here) | Not in the item; `test 42` asserts it: login, settings stay shared |
| The HANDOFF cites `compose.hr.yaml:99-114` | STALE | The `~/.claude` bind sits lower in the file (the volumes list moved); cite by content, not line |
| Read-only `plugins/` degrades quietly (auto-update, `.last_inuse_sweep`, `installed_plugins.json`, directory caches) | **UNVERIFIED** | T0 probes it before any compose change |
| A nested rw bind under a ro bind works in Docker Compose | **UNVERIFIED** | T0 / D2 docker test; Docker allows it (separate mounts) but this sandbox has no Docker, so the real check is host-owed |
| `$.store` degrades quietly when `plugins/store` is rw | n/a | It is excluded from the ro mount, so nothing changes |

## Decisions for the maintainer

- **D1 - scope vs `settings.json` (the important one).** Read-only `plugins` + `skills` does not make a run
  harmless: `~/.claude/settings.json` stays rw (login and `/model` need it), so a hostile run can still write
  `enabledPlugins` plus an `extraKnownMarketplaces` directory source pointing at another shared rw path
  (`~/.headroom`, `~/.serena`, `~/.cache/uv`, the run's project dir), or a settings hook. The threat model
  already lists "add hooks or settings", so this is a known class; TOOL-10 only removes the *direct* mod routes.
  **Recommend:** ship TOOL-10 as specified, state the residual in the docs, and file a follow-up item
  (settings.json ro or per-run copy, plus `agents/`, `commands/`, `hooks/`) rather than widen this one.
- **D2 - `dev-mods`: tmpfs mask, not ro.** A read-only `dev-mods` would break a run's own mod authoring
  (Claude writes there); an empty 16m tmpfs keeps that working inside the container and discards it on stop,
  so nothing reaches a later host session. Recommend tmpfs, same as `sessions`/`jobs`.

## Patterns to Mirror

| Category | Source | Pattern |
|---|---|---|
| Naming / compose | `docker/hr/compose.hr.yaml` `~/.claude` block | one-line `- {type: ..., target: ...}` entries with a trailing comment, parents before children, every tmpfs sized |
| Mount points on host | `scripts/hr` `prepare_claude_mounts`, `CLAUDE_MASK_DIRS` | `mkdir -p` each mount point as the user, so Docker does not create it root-owned in the host `~/.claude` |
| Tests (contract) | `tests/hr/hr.bats` test 42, `hr_helper.bash` `HR_CLAUDE_MASK_DIRS`/`HR_CLAUDE_TMPFS_SIZE` | `compose_config` + `jq` over `.services.dev.volumes`; the count assertion pins "nothing else under `~/.claude`" |
| Tests (host mounts) | `tests/hr/hr.bats` test 43 | mount points exist, owned by the user |
| Tests (docker) | `tests/hr/hr.bats` ~1735-1765 (D2) | `dexec` into a real run, `findmnt -no OPTIONS` |
| Docs | `docs/hr-sandbox.md` "What is inside a sandbox" table + "Threat model" table + "Known residuals" | one row per mount; threat-model row says what a hostile run can do |

## Files to Change

| File | Action | Why |
|---|---|---|
| `docker/hr/compose.hr.yaml` | UPDATE | `plugins` and `skills` ro binds; `plugins/data`, `plugins/store` rw binds over `plugins`; `dev-mods` tmpfs 16m |
| `scripts/hr` | UPDATE | `CLAUDE_MASK_DIRS` += `dev-mods`; `prepare_claude_mounts` also creates `plugins/data`, `plugins/store`, `skills`, `plugins` |
| `tests/hr/hr_helper.bash` | UPDATE | `HR_CLAUDE_MASK_DIRS` += `dev-mods`; new arrays for the ro dirs and the rw-under-ro dirs |
| `tests/hr/hr.bats` | UPDATE | test 42 (contract + new count), test 43 (mount points), D2 docker test (`findmnt` ro; a write to `plugins/` fails, to `plugins/data` works) |
| `docs/hr-sandbox.md` | UPDATE | the two tables, the threat-model row, residuals, a troubleshooting line |
| `HANDOFF.md`, `DECISIONS.md`, `docs/decisions/tool/tool-10.md` | UPDATE / CREATE | close-out per `lifecycle.md` P2 |

## Tasks

Serial. The file sets are not independent in effect: T2's contract test (T1) pins what T3 builds, and T4's
wording depends on T0's findings. No fan-out, no ultracode.

### Task 0: probe (no repo change)
- **Action**: scratch `CLAUDE_CONFIG_DIR` under `/tmp` (per the live-probe memory: credentials minus
  `refreshToken`; never the shared `~/.claude`). Copy `plugins/` and `skills/`, `chmod -R a-w` them except
  `plugins/data` and `plugins/store`, run `claude -p` and a session that loads a plugin with a `$.store` call,
  and read stderr and the debug log for write errors, hangs or noise. Record plugin auto-update behaviour.
- **Validate**: result table in the write-up. If a write error is loud or the session hangs, stop and bring it
  back before T2 (the item's "degrade quietly" is a gate, not a hope).
- **Limit**: no Docker in the sandbox, so the nested ro/rw mount itself is T5 on the host.

### Task 1: contract tests first (red)
- **Action**: update `hr_helper.bash` and test 42/43 as above; the count becomes
  `MASK_DIRS + MASK_FILES + 1 + (2 ro binds) + (2 rw-under-ro binds)`. Assert: `plugins`, `skills` bind
  `ro: true`; `plugins/data`, `plugins/store` bind `ro: false`, listed **after** `plugins`; `dev-mods` is a
  sized tmpfs; `settings.json`/`.credentials.json` are *not* separately mounted.
- **Mirror**: test 42.
- **Validate**: `bats --filter-tags '!docker' tests/hr` fails on exactly 42/43.

### Task 2: compose + scripts/hr (green)
- **Action**: edit `compose.hr.yaml` (comment the why: "a plugin or skills-dir plugin written here runs in
  process in the host's next session - TOOL-10") and `scripts/hr`.
- **Validate**: `bats --filter-tags '!docker' tests/hr` (85 cases) green; `shellcheck scripts/hr`.

### Task 3: docker test
- **Action**: extend D2 in `hr.bats`. Runs only where Docker exists.
- **Validate**: host-owed (T5).

### Task 4: docs
- **Action**: in `docs/hr-sandbox.md`: split the `~/.claude` row of the "What is inside" table (plugins/skills
  ro, data/store rw, dev-mods tmpfs); change the threat-model row from "add hooks or settings..." to say plugins
  and skills are read-only and that `settings.json` hooks and `enabledPlugins` still are the route; add
  residuals (settings.json, `agents/`, `commands/`, plugin-data code, the dangling `find-skills` link); a
  troubleshooting entry: "a plugin install/update fails in a run" -> do it on the host.
  Update `README.md` "Running several /handoff-run sessions" only if it repeats the old row.
- **Validate**: `bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh` 0/0.

### Task 5: close-out and host gate
- **Action**: P2 bookkeeping; reviewer gate (`rust-reviewer` is the configured reviewer but this change has
  no Rust: say so to the maintainer and ask whether to run it on the bash/compose diff or skip).
  Done-report ends `branch hr/TOOL-10 ready — on host: scripts/hr collect TOOL-10`.
- **Host-owed**: `scripts/hr up` on a real run, D2 docker test, a plugin install attempt inside a run.

## Validation

```bash
bats --filter-tags '!docker' tests/hr
shellcheck scripts/hr
bash .claude/skills/handoff-run/scripts/validate-workflow-docs.sh
```

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| ro `plugins/` makes Claude noisy or slow at startup (cache refresh, sweep) | Medium | T0 probe before any change; fall back to masking only `installed_plugins.json`+`marketplaces` if needed |
| Nested rw bind under ro bind is refused by an older Docker | Low | Compose docs require Docker 26+; D2 docker test; host-owed |
| Missing mount point makes Docker create `plugins/data` root-owned in the host `~/.claude` | Medium | `prepare_claude_mounts` creates them first (test 43) |
| Runs lose `skills/learned` writes (continuous-learning) | Low | Accepted; documented. The dir is empty today |
| False sense of safety: `settings.json` is still rw | High | D1: state it in the docs and file the follow-up |
| `plugins/data` becomes an exec route if a plugin puts a venv there | Low (empty today) | Residual row in the threat model |

## Acceptance

- [ ] T0 probe table in the write-up; no loud failure under a read-only `plugins/`
- [ ] Contract tests pin the mounts; `bats --filter-tags '!docker' tests/hr` green
- [ ] `docs/hr-sandbox.md` tables and threat model updated, residuals named
- [ ] Validator 0/0; close-out docs written
- [ ] Host-owed items listed in the done-report (real `up`, D2 docker, install attempt)

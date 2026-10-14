# TOOL-10 - hr sandbox: stop a run from planting a mod that runs on the host (done, 2026-10-14)

From ANA-29 §3.3. Plan: `.claude/plans/tool-10-sandbox-mod-hardening.plan.md`. Commit `6ed400a3`.

## Built

A run mounted `~/.claude` read-write and shared, so a plugin it installed, or a
`~/.claude/skills/<x>/.claude-plugin` it wrote (loads as `@skills-dir`), would run in process in the
host's next Claude session. In `docker/hr/compose.hr.yaml` the `dev` service now has:

- `~/.claude/plugins` and `~/.claude/skills` as **read-only** binds;
- `~/.claude/plugins/data` (`${CLAUDE_PLUGIN_DATA}`) and `~/.claude/plugins/store` (`$.store` files) bound
  back **read-write**, listed after the `plugins` bind;
- `~/.claude/dev-mods` as an empty 16 MB tmpfs, like `sessions` and `jobs`.

`scripts/hr` creates the new mount points as the user before `up` (`CLAUDE_RO_DIRS`,
`CLAUDE_RW_UNDER_RO`, `dev-mods` in `CLAUDE_MASK_DIRS`), so Docker never makes them root-owned in the host
`~/.claude`. `tests/hr` pins the contract (test 42: modes, order, the mount count, no separate mount for
`settings.json` or `.credentials.json`; test 43: mount points; the Docker case: a write to `plugins` and
`skills` fails, to `plugins/data` and `plugins/store` works). `docs/hr-sandbox.md` and the README line say
what changed.

## Decisions

- **`dev-mods`: a tmpfs, not read-only.** A mod Claude writes there loads only in the session that wrote it,
  after a hot-reload approval, in a trusted workspace. It reaches the host only if the host resumes that run
  session, which it can: the clone sits at the host repo's path, so the run's transcripts are in the shared
  project dir, and the approval "lasts for the session, including after you resume it". A read-only mount
  would break mod authoring in a run; a tmpfs keeps it and discards the result.
- **Scope.** `settings.json` stays read-write (login and `/model` need it). A hostile run can still set
  `enabledPlugins` and a directory marketplace, or add a settings hook. This item closes the direct mod
  routes only. The rest is filed as TOOL-12.

## Verified

- Probe on a scratch `CLAUDE_CONFIG_DIR` with `plugins/` and `skills/` `chmod a-w` (data and store writable):
  `claude -p` exits 0, stderr empty. The only trace is debug-level `Failed to write .orphaned_at` for each
  cached plugin version. Plugin auto-update was not exercised; a plugin install or update inside a run now
  fails, and the docs say to do it on the host. `plugins/store` is excluded from the read-only mount, so
  `$.store` is unchanged.
- The compose file parses; the new mounts' flags and order, and the count of 22 mounts under `~/.claude`,
  match test 42 (checked by running its `jq` assertions on the parsed YAML, since the sandbox has no `docker`).
- `bats --filter-tags '!docker' tests/hr`: 12 cases fail, the same 12 on a clean `HEAD` (they need `docker` or
  the host: 3g, 3h, 3l, 3r, 21, 25, 26, 32, 40, 40b, 42, 44). Test 43 passes. Test 42 needs `docker compose config`
  and could not run here.

## Owed on the host

`scripts/hr build`/`up` on a real run (nested read-write binds under a read-only bind), the Docker case
(`bats --filter-tags docker tests/hr/hr.bats`), test 42 with `docker compose config`, and a plugin install
attempt inside a run.

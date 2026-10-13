# Blueprint: MOD-93, the `htui` Claude Code plugin emitted by `htui claude emit`

**Plan:** `.claude/plans/mod-93-htui-claude-plugin.plan.md` (approved, D1-D15). Produced by `code-architect`.
Rust line numbers are for `39a72e1f`; anchor on names once commits land. Mods-API citations are
`claude-code.d.ts` as Claude Code 2.1.296 wrote it (`plugin-authoring/types/`), shortened to `d.ts:N`.

Waves are unchanged: W1 = T1 ∥ T2, W2 = T3 ∥ T4. Every claim marked **probed** was run in a scratch
copy of the probe (`/tmp/mod93-arch`, scratch `CLAUDE_CONFIG_DIR`) under `claude plugin validate
--strict` and `claude plugin test` on 2.1.296.

---

## Plan amendments and corrections

**PLAN AMENDMENT NEEDED** marks a change to the plan's text. The other items fill gaps the plan
leaves open.

- **A-1 (PLAN AMENDMENT NEEDED, Files table `hooks/*.test.ts`). The kit tests go in
  `crates/htui/claude-plugin/htui/tests/`, not in `hooks/`.** Three test files, one fixture module
  (`tests/fixtures.tsx`). Probed: `claude plugin test` finds `*.test.ts` and `*.test.tsx` under the
  folder recursively, a test imports `./fixtures`, and `validate --strict` passes with `tests/`
  present. Two reasons. The tests share helpers (A-3 needs a JSX sentinel in every render test).
  Also T3's walk test can then leave out one directory, rather than matching file names.
  **This couples T2 and T3; see "Coupling".**
- **A-2 (PLAN AMENDMENT NEEDED, D15 "every other hook returns `next(e)`").** The `command.run`
  hook for `htui` must **never read `next`**. Probed: with a literal `$.command.register({ name:
  'htui' })` in the module and no `next`, validate reports `answers its own command:
  command.run{command=htui}`. Remove the register call or read `next`, and it becomes `gating hook
  without .catch: command.run{command=htui}` (also probed). The hook cannot run in a stand-down
  session anyway, because `/htui` is never registered there. The `ui.render` hooks keep `next(e)`.
- **A-3 (new fact, Mod tests).** The kit has nothing beneath `ui.render`. If the band passes
  (`next(e)`), `$.ui.mount` fails with `HooksError: $.ui.mount: no implementation for ui.render`
  (probed). Answering `null` from a test hook is refused ("not a tree element", probed). Every test
  that expects the band or pane to pass therefore registers a sentinel bottom. It then asserts that
  the sentinel is found and the mod's keys are not:
  `on('ui.render', { component: 'AbovePrompt' }, async ($, e) => { const { Box } = $.ui.resolve(e); return <Box key="engine" /> })`.
  This needs JSX, so those tests are `*.test.tsx` (probed: passes).
- **A-4 (PLAN AMENDMENT NEEDED, T2 types `isPaneOpen`).** Drop `isPaneOpen` from `$.state`. Whether
  the pane is open is read from `$.ui.panes()`, "the engine's record, not the module's: a module
  reloaded while its pane stayed up finds it here" (`d.ts:2521-2533`). The poll timer checks it on
  each tick, and stops itself in `command` mode when the pane is gone (probed: the timer stops,
  `cancel()` holds). There is no `ui.close` hook. Probed: the hook works, but validate lists it as
  `gating hook without .catch: ui.close{id=htui}`, and the test `$` has no `ui.close`
  (`TypeError: $.ui.close is not a function`), so the hook could not be tested.
- **A-5 (refines D13).** A `$.process.run` rejection does not say which failure it was
  (`d.ts:3505-3522`: "Rejects when the command cannot start or is still running then"). The mod
  reads `$.clock.now()` before the call and in the `catch`. Elapsed ≥ `timeoutMs` gives `htui: timed
  out`; anything else gives `htui: not found on PATH`. Probed both: a test hook answering `{ deny }`
  at once, and one that answers after `await clock.sleep(10000)` while the test runs
  `clock.advance(10000)`.
- **A-6 (refines T2 "search argv").** The argv is `['htui', `--search-items=${query}`, '--json']`.
  The query rides inside one argument after `=`. Written as two arguments, a query that begins with
  `-` would be read by clap as a flag (a usage error, exit 2, empty stdout). The test asserts the
  exact argv and `e.init.timeoutMs === 60000`.
- **A-7 (PLAN AMENDMENT NEEDED, T1 "link `docs/htui-cli.md` by its repository URL").** No browsable
  URL is verified. The workspace's `repository = "https://git.mluigi.it/htui.git"` (`Cargo.toml:10`)
  is a clone URL. ANA-29 §5.4 U1 marks anonymous readability of `git.mluigi.it` as **UNVERIFIED**,
  and the GitHub mirror's currency is unknown. Default, pending the maintainer: SKILL.md states the
  schema subset it needs, every fact traceable to `docs/htui-cli.md`. It names the doc as "`docs/
  htui-cli.md` in htui's repository (`https://git.mluigi.it/htui.git`)" and adds no web link. If the
  maintainer names a URL, it replaces that line. The skill runs offline either way.
- **A-8 (correction, plan "Verified claims", d.ts cites).** The lines drift. `$.clock.every` is
  `d.ts:3471` (the plan says 3469). `$.env.get` is `:3605` (plan: 3595). The `argumentHint` that
  `$.command.register` takes is `CommandSpec`'s at `:1905`; the plan's `:1708` is
  `CommandDescribeInput`. `CommandRunResult.text` is `:1844` (plan: 1843). `$.process.run` at `:3522`
  is right.
- **A-9 (correction, D1 "fails a unit test (D10)").** The table-drift test is a **unit test in
  `claude_emit.rs`**. D10 is the gate test in `tests/claude_plugin.rs`. Both exist, and the walk is
  not in D10.
- **A-10 (gap).** The engine writes `.claude-plugin/types/` into a plugin folder it hot-reloads,
  such as `claude --plugin-dir crates/htui/claude-plugin/htui` (reference.md, "The types are the
  reference"). T2 adds `/crates/htui/claude-plugin/htui/.claude-plugin/types/` to the root
  `.gitignore`. T3's walk test leaves out that directory.
- **A-11 (gap, D6).** `capture_version` is not re-exported at the crate root. Call it as
  `htui_agent::probe::capture_version`, with `htui_agent::VersionProbe` (re-exported from `launch`).
  It takes a `path`. A bare `claude` works because `launch::spawn` resolves the command with
  `which::which` on the **process** `PATH` (`htui-agent/src/launch.rs:1088-1095`), not on
  `ProbeEnv.vars`. "Not on PATH" and "printed no version" both come back as `None`. D6 treats them
  alike, so there is no gap.
- **A-12 (gap, D12).** `userConfig` has no minimum for numbers. Out-of-range values are clamped in
  code: `Math.max(5, Number(options.poll_seconds) || 30)`. Probed: a `panel` value outside
  `options` reaches `register` as the default `auto`, and `poll_seconds` arrives as a `number`.

The rest of the plan's claims check out: `cli.rs:120-144/173-184`, `lib.rs:99-128`,
`main.rs:103-152`, `StateExit` at `state_cmd.rs:50-83`, `below_min` at `:1017`, `capture_version`
at `:1118`, `ProbeEnv::host` at `:99`, `default_install_root` at `:204` (the plan says 205), `SESSION_VAR` at
`launch.rs:607`, `dirs = "6"` (`Cargo.toml:91`, one `dirs 6.0.0` in the lock, not yet in
`crates/htui/Cargo.toml`), and the `#![cfg(feature = "testkit")]` on `tests/waiting.rs:10`. Also
probed: `0.0.0-source` validates under `--strict`; `hooks/version.ts` imported by `register.tsx`
validates and loads; a skill-only `plugin.json` that still names `types` fails `--strict` ("Path
not found"); `validate` on the marketplace root checks only `marketplace.json`, so both commands
are needed; `validate --json` reports `contents[].notes` (the `calls:` line) and `gatingHooks`.

---

## File sets and coupling

| Task | Files (exact) |
|---|---|
| T1 | `crates/htui/claude-plugin/.claude-plugin/marketplace.json`, `crates/htui/claude-plugin/htui/.claude-plugin/plugin.json`, `crates/htui/claude-plugin/htui/skills/htui/SKILL.md` |
| T2 | `crates/htui/claude-plugin/htui/hooks/hooks.json`, `…/htui/hooks/register.tsx`, `…/htui/hooks/version.ts`, `…/htui/types/index.d.ts`, `…/htui/tests/fixtures.tsx`, `…/htui/tests/lifecycle.test.tsx`, `…/htui/tests/commands.test.ts`, `…/htui/tests/render.test.tsx`, `.gitignore` (one line, A-10) |
| T3 | `crates/htui/src/claude_emit.rs` (new), `crates/htui/src/cli.rs`, `crates/htui/src/lib.rs`, `crates/htui/src/main.rs`, `crates/htui/Cargo.toml`, `Cargo.lock`, `crates/htui/tests/claude_plugin.rs` (new) |
| T4 | `docs/htui-claude-plugin.md` (new), `docs/htui-cli.md`, `README.md` |

T1 ∩ T2 = ∅ and T3 ∩ T4 = ∅ (checked path by path).

**Coupling: T3 depends on these names and texts, so T1 and T2 must deliver them exactly:**
1. The seven embedded paths, relative to `crates/htui/claude-plugin/`. If one is renamed, both the
   T3 table and its walk test change.
   - `.claude-plugin/marketplace.json` (T1)
   - `htui/.claude-plugin/plugin.json` (T1)
   - `htui/skills/htui/SKILL.md` (T1)
   - `htui/hooks/hooks.json` (T2)
   - `htui/hooks/register.tsx` (T2)
   - `htui/hooks/version.ts` (T2)
   - `htui/types/index.d.ts` (T2)
2. Two directories the walk leaves out: `htui/tests/` (A-1) and `htui/.claude-plugin/types/` (A-10).
   **T2 adds no other file under `crates/htui/claude-plugin/`.** A new module (say `hooks/format.ts`)
   would be a new table row in T3. If T2 needs one, it says so before W2 starts.
3. `version.ts` is exactly `export const VERSION = '0.0.0-source'\n`, single quotes, the literal
   appearing once.
4. `plugin.json` has top-level `"version": "0.0.0-source"`, `"types"` and `"userConfig"`, and
   nothing in the plugin depends on key order (T3 rewrites with `serde_json`, which sorts keys).
5. `marketplace.json` is an object with `"name": "htui"`, which the ownership check reads.

T4 copies the mod's visible strings from **§T2 Text formats** below, not from T2's code. It copies
flags and exit codes from §T3.

---

## T1: manifests and skill

- `marketplace.json`: as in the probe, pretty-printed:
  `{ "name": "htui", "owner": { "name": "htui" }, "description": "htui's plugin, written by htui claude emit", "plugins": [ { "name": "htui", "source": "./htui", "description": "Read htui's state from Claude Code and hand work to htui" } ] }`.
- `plugin.json`: `name: "htui"`, `version: "0.0.0-source"`, `description`, `author: { name: "htui" }`,
  `types: "./types/index.d.ts"`, and `userConfig.panel` and `userConfig.poll_seconds` as in the probe.
  The descriptions are D12's sentences: `panel`: "auto: a band above the prompt when something waits, and /htui opens a pane; command: no band, /htui opens the pane; off: /htui prints the rows as text". `poll_seconds`: "Seconds between htui waiting polls while the band or pane shows (at least 5)".
- `SKILL.md`: frontmatter `name: htui` and a `description` that triggers on "htui", "waiting on me",
  "what is htui doing", "hand this to htui" and "is there an htui item for". The body, in order:
  1. when to use it;
  2. the three commands with `--json` (`htui status --json`, `htui waiting --json`, `htui --search-items "<q>" --json`), with the scope default (first workspace) and `--project`;
  3. the envelope (`version`, `kind`, `store`) and "ignore unknown fields, treat an unknown value as something else";
  4. exit codes 0/3/2/1, `error.code` and `error.message`, and "empty stdout is a failure, whatever the exit code";
  5. how to read a `waiting` row (`reason` values, `item_key`, `step`, `text`, `age_seconds`);
  6. the hand-off rules exactly as the plan's T1 bullet gives them;
  7. "no write command exists; never connect to htui's Postgres or Qdrant";
  8. the doc reference, per A-7.
- **TDD/commit**: there are no tests. Validate `claude plugin validate --strict crates/htui/claude-plugin`
  (the marketplace only). The plugin fails until T2's `types/index.d.ts` exists (probed), which is
  expected in W1. Commit `feat(mod-93): plugin manifests and the htui skill` with the three paths
  staged by name.

---

## T2: the mod

### `$.state` contract (`types/index.d.ts`, exact)

```ts
/** One row of `htui waiting --json` as the mod reads it (docs/htui-cli.md, "waiting"). Other fields are ignored. */
export type HtuiRow = { reason: string; item_key: string; step: string | null; text: string; age_seconds: number }

/** The last poll: its rows, or the visible line for a poll that failed (then `rows` is empty). */
export type HtuiWaiting = {
  rows: readonly HtuiRow[]
  /** `workspace.name` as the top bar shows it, or null when the store has no workspace. */
  workspace: string | null
  /** `$.clock.now()` when the poll answered. */
  at: number
  error: string | null
}

declare module 'claude-code' {
  interface PluginState {
    htui: {
      /** null until the first poll answers. */
      waiting: HtuiWaiting | null
      /** D14's skew line, or null. */
      skew: string | null
      /** True once session.start ran and did not stand down (D15); the render hooks read it. */
      active: boolean
    }
  }
}
```

Atoms in `register.tsx`, following `band.tsx`:
`atom({ plugin: 'htui', key: 'waiting' } as const, null as HtuiWaiting | null)`, and `skew` and
`active` likewise (`d.ts:14709-14743`; `read` inside `ui.render` subscribes the drawing, so a poll
redraws the band).

### `register.tsx`: module shape

- Imports: `atom, read, update` (values) and `type Register, Timer, ProcessRunResult` from
  `'claude-code'`; `type { HtuiRow, HtuiWaiting } from '../types'`; `{ VERSION } from './version'`.
  No `import()` (reference.md).
- `register: Register = (on, options)`. Options are parsed once:
  `panel = options.panel === 'command' || options.panel === 'off' ? options.panel : 'auto'`, and
  `periodMs = Math.max(5, Number(options.poll_seconds) || 30) * 1000` (`PluginOptions` at
  `d.ts:7666`, `Register` at `:9309`).
- Module state: `let timer: Timer | null = null` (`d.ts:12623`, `{ cancel }`) and
  `let polling = false`, a guard so a 10 s poll never overlaps a 5 s period.
- Helpers, all inside the module:
  - `runHtui($, argv, timeoutMs): Promise<{ ok: true; doc: any } | { ok: false; line: string }>`
    takes these steps in order:
    1. A rejection gives `htui: timed out` or `htui: not found on PATH` (A-5).
    2. `stdout.trim() === ''` gives `htui printed nothing (exit N)`.
    3. A `JSON.parse` that throws, or a result that is not an object, gives `htui: unreadable answer (exit N)`.
    4. `doc.version > 1` gives `htui: answer version V is newer than this plugin: run htui claude emit --force, then /reload-plugins`.
    5. A `doc.error?.message` that is a string gives `htui: <message>`. This covers exits 3, 2 and 1, and a search's `index` failures.
    6. `exitCode !== 0` gives `htui: exit N`.
    7. Otherwise `{ ok: true, doc }`.

    It never throws.
  - `poll($)`: does nothing while `polling` is set. It runs `['htui', 'waiting', '--json']` with
    `{ timeoutMs: 10000 }` and writes `waiting` as `{ rows: doc.rows, workspace: doc.workspace?.name ?? null, at, error: null }`
    or `{ rows: [], workspace: null, at, error: line }`. On a failure it never keeps the old rows,
    so the band never shows stale data as live (htui-cli.md, "Exit codes").
  - `startTimer($)`: does nothing if a timer runs. Otherwise
    `timer = $.clock.every(periodMs, () => void tick($))` (`d.ts:3462-3471`). In `tick`, when
    `panel === 'command'` and `!(await $.ui.panes()).some(p => p.id === 'htui')`, it runs
    `timer?.cancel(); timer = null` and returns. Otherwise it calls `poll($)`.
  - `checkVersion($)`: runs `['htui', '--version']` with `{ timeoutMs: 10000 }`. On exit 0 it parses
    `/^htui (\S+)/m`, and if the result differs from `VERSION`, it sets `skew` to
    `` `htui ${bin} ≠ plugin ${VERSION}: run htui claude emit --force, then /reload-plugins` ``.
    A rejection or no match leaves `skew` null. The poll reports a missing binary.
  - `age(s)`: `42s`/`5m`/`3h`/`2d`, floor division, as in `state_cmd.rs:964` `age_text`.
    `label(reason)`: `_` becomes a space (`judge failed`).

### Hooks and lifecycle

| Hook | Matcher | Behaviour |
|---|---|---|
| `session.start` | none | If `await $.env.get('HTUI_SESSION') !== undefined`, return `next(e)` at once: no register, no timer, `active` stays false (D15). Otherwise `await $.command.register({ name: 'htui', description: "htui: what waits on you, status, item search", argumentHint: '[status\|waiting\|search <query>]' })` (`d.ts:3098`, `CommandSpec` `:1892`), then `update(active → true)`. Then `void checkVersion($)`. When `panel === 'auto'`, also `void poll($)` and `startTimer($)`. Return `next(e)`. Process runs are detached (`void`) so the hook never holds its 10 s budget on a slow `htui` (`d.ts:4330-4341`: the first `session.start` is awaited before turn one). A reload of a changed module raises `session.start` again (`d.ts:4335-4336`), which restarts the timer after `emit --force` + `/reload-plugins`. |
| `command.run` | `{ command: 'htui' }` | Never reads `next` (A-2). Routing on `e.args.trim()` (`CommandRunInput.args` at `d.ts:1805-1809`): split on the first whitespace into a verb and the rest. The verbs: `''`, `status`, `waiting`, `search` with a non-empty rest, and anything else. **`''`**: when `panel === 'off'`, run the waiting text below. Otherwise `const o = await $.ui.open({ id: 'htui', title: 'htui' })` (`d.ts:2506`), `void poll($)`, `startTimer($)`, and return `{}` when `o.isPlaced`, else `{ text: `htui: the pane waits: ${o.reason}` }` (`UiOpenResult` at `d.ts:14142-14166`). **`status`, `waiting`, `search <q>`** return `{ text }` (formats below). **Anything else** returns `{ text: 'usage: /htui [status\|waiting\|search <query>]' }`. |
| `ui.render` | `{ component: 'AbovePrompt' }` | Return `next(e)` when any of these holds: `panel !== 'auto'`, `!active`, `e.props.hasSurvey` (`d.ts:10267`), or nothing to show (`waiting` null or with no rows and no `error`, and `skew` null). Otherwise draw the band tree. |
| `ui.render` | `{ component: 'Pane', requestId: 'htui' }` | Always draws (the person opened it). It reads `e.props.bodyColumns` (`d.ts:10338`). |

Polling summary (D12):
- `auto`: one poll at `session.start`, then every `periodMs` for the session.
- `command`: from the first `/htui` until a tick finds the pane closed.
- `off`: never. Each `/htui` runs one `waiting` call.
- A module reload drops the timer (reference.md, "Developing one"); `session.start` restarts it.

### Trees (`Text` drops `key` in a mounted tree, so keys go on `Box`es; `BoxProps.key` is at `d.ts:916`, and `TextProps` at `:12571` has none)

- **Band**: `<Box key="htui-band" flexDirection="column">`, holding in order and only when present:
  - `<Text wrap="truncate">htui: {n} waiting on you</Text>`, when n > 0;
  - `<Text wrap="truncate">{skew}</Text>`;
  - `<Text dimColor wrap="truncate">{error}</Text>`.

  The band's width is the engine's `bodyColumns` (`d.ts:10292`). `truncate` keeps each line to one row.
- **Pane**: `<Box key="htui-pane" flexDirection="column">` holding:
  - a header `<Text bold wrap="truncate">{workspace ?? 'no workspace'} · {n} waiting</Text>`;
  - the `skew` and `error` lines when present;
  - one `<Box key={`row-${i}`}><Text wrap="truncate">{line}</Text></Box>` per row;
  - or `<Text dimColor>Nothing waits on you.</Text>`;
  - or `<Text dimColor>htui: no answer yet</Text>` while `waiting` is null.

  A row line is `fit(`${label(reason).padEnd(13)}${item_key.padEnd(10)}${(step ?? '-').padEnd(16)}${age(age_seconds).padStart(4)}  ${text}`, bodyColumns)`,
  where `fit` cuts to `bodyColumns` with a trailing `…`.

### Text formats (T4 copies these)

| Command | Success text | Failure text |
|---|---|---|
| `/htui status` | `{workspace.name ?? 'no workspace'} · {box?.hostname ?? 'no box'} · {store} · {working} working · {waiting} waiting`, the CLI's text line (htui-cli.md, "Text output"). Then `permission requests could not be listed; the count may be low` when `permissions_known` is false. Then the skew line, if any | `runHtui`'s line, then the skew line, if any |
| `/htui waiting` (and `/htui` under `panel=off`) | One line per row: `{item_key}  {label(reason)}  {step ?? '-'}  {age}  {text}`. No rows: `Nothing waits on you.` | `runHtui`'s line |
| `/htui search <q>` | One line per hit: `{key}  {place}{document_kind ? ' ' + document_kind : ''}{resolution ? ' (' + resolution + ')' : ''}  {snippet}`. `index: "empty"`: `No concepts index yet: run htui --index-items.` No hits: `No hits.` | `runHtui`'s line |
| `/htui search` (no query) | `usage: /htui search <query>` | |

### Kit tests (`tests/`, A-1). Mock rules from the probe and `d.ts:14882-15967`

- `fixtures.tsx` exports these helpers:
  - `ok(stdout, exitCode = 0)`, which returns `{ value: { exitCode, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }` (`ProcessRunResult` at `d.ts:8034`);
  - `htui(on, answers: Record<string, ReturnType<typeof ok> | 'deny' | 'hang'>, calls: { argv: string[]; timeoutMs?: number }[])`, which registers `on('process.run', async ($, e) => …)`. It keys on `e.argv[1]` (`'waiting'`, `'status'`, `'--version'`, or a `--search-items=` prefix) and records `e.argv` and `e.init?.timeoutMs` (`'process.run'` op args at `d.ts:7160`). `'deny'` answers `{ deny: 'spawn htui ENOENT' }`. `'hang'` waits `await clock.sleep(10000)` and then answers `{ deny: 'timed out' }`;
  - `start($, on, env = {})`, which runs `mock.env(on, env)`, `on('command.register', async () => ({ value: undefined }))`, `on('session.start', async ($, e) => e)`, then `await $.session.start({ source: 'startup', cwd: '/tmp' } as never)`;
  - `engineBottom(on, component)`, the A-3 sentinel;
  - `BAND = { hasSurvey: false, isWorking: false, maxRows: 10, bodyColumns: 80 } as never` and `PANE = { title: 'htui', isFocused: false, bodyColumns: 60, placement: 'inline' } as never`;
  - JSON fixtures copied from htui-cli.md's examples: a status, a waiting with two rows, a waiting with no rows, offline, `no_dsn`, and a search with two hits or with `index: "empty"`.
- Each test calls `mock.clock(on)` first, so `$.clock.every`, `now` and `sleep` are held. UI tests loop
  `for (const surface of ['terminal', 'desktop'] as const)` and use
  `$.ui.mount({ plugin: 'htui', surface, component, props, requestId? })` (`MountTarget` at
  `d.ts:15808`), then `find`/`findAll` by `{ key }` or `{ type: 'Text', text: /re/ }`
  (`d.ts:15621-15630`), then `unmount()`. Options come from `test(name, { options: { panel, poll_seconds } }, body)` (`TestOptions` at `d.ts:15947`).

| File | Test (sentence names) | Mocks |
|---|---|---|
| `lifecycle.test.tsx` | `auto polls at start and every poll_seconds` | `htui` counts `waiting` calls: 1 after `start`, +1 after `clock.advance(30000)` |
| | `poll_seconds below 5 is read as 5` | `{ options: { poll_seconds: 1 } }`; advance 5000 adds +1 call, advance 1000 adds none |
| | `command polls nothing until /htui and stops when the pane closes` | `{ options: { panel: 'command' } }`; `on('ui.open', …{ value: { isPlaced: true } })`, `on('ui.panes', …)` answers from a mutable flag; 0 `waiting` calls after advance; `/htui` adds 1; advance adds 1; the flag goes false; two advances add none (probed shape) |
| | `off never polls and never opens a pane` | `{ options: { panel: 'off' } }`; a `ui.open` hook that counts; after `/htui` the text is the rows, there was 1 `waiting` call, and `ui.open` was called 0 times |
| | `an htui-driven session stands down` | `start($, on, { HTUI_SESSION: '1' })`; the `command.register` counter stays 0; no process call after advance; the band mount finds `engine`, not `htui-band` |
| | `a slow poll is not overlapped` | the `waiting` answer is `'hang'`, `poll_seconds: 5`; advance 5000 twice, and the `waiting` calls stay 1 until the hang resolves |
| `commands.test.ts` | `status prints the top bar's line` | the status fixture gives `Platform · desk · online · 1 working · 2 waiting` |
| | `status shows the skew line` | `--version` gives `ok('htui 9.9.9\n')`; after `start`, the text holds `htui 9.9.9 ≠ plugin 0.0.0-source: run htui claude emit --force, then /reload-plugins` |
| | `waiting prints one line per row, and a sentence when none` | |
| | `search sends the query as one argument with a 60 s bound` | `calls[0]` equals `{ argv: ['htui', '--search-items=closed items', '--json'], timeoutMs: 60000 }`; hit lines |
| | `search on an empty index says so` / `search with no query prints usage` / `an unknown verb prints usage` | |
| | `an offline store exits 3 and shows its message` | `ok(OFFLINE, 3)` gives `htui: cannot reach Postgres: …` |
| | `a refusal exits 2 and shows its message` | `ok(NO_DSN, 2)` |
| | `a missing binary is a line, not a throw` | `'deny'` gives `htui: not found on PATH` |
| | `a hung binary is a line, not a throw` | `'hang'`; run unawaited, `clock.advance(10000)`, then `htui: timed out` |
| | `empty stdout is a failure whatever the exit code` | `ok('', 0)` gives `htui printed nothing (exit 0)` |
| `render.test.tsx` | `the band shows the count on terminal and desktop` | two rows; mount `AbovePrompt` with `BAND`; find `{ type: 'Text', text: /2 waiting on you/ }` and `{ key: 'htui-band' }` |
| | `no rows and no skew leaves the band to the engine` | `engineBottom`; find `engine`, not `htui-band` |
| | `a survey keeps the band away` | `{ ...BAND, hasSurvey: true }` |
| | `a failed poll shows its line in the band` | offline fixture |
| | `command and off draw no band` | `{ options: { panel } }` × 2, `engineBottom` |
| | `the pane lists rows cut to its width` | `PANE` with `bodyColumns: 40`, `requestId: 'htui'`; `findAll({ type: 'Text' })` gives every row text ≤ 40 characters; `row-0` and `row-1` are found |
| | `an empty pane says nothing waits` | |

### T2 TDD order and commits (each commit stages its own paths by name)

1. `types/index.d.ts`, `version.ts`, `hooks.json`, the `.gitignore` line, `tests/fixtures.tsx`, and
   `tests/lifecycle.test.tsx` (red), then `register.tsx` up to session.start/timer/stand-down, to
   green under `claude plugin test`. Commit `feat(mod-93): the mod's lifecycle - state contract, poll timer, stand-down`.
2. `tests/commands.test.ts` (red), then `runHtui` and the four command branches, to green. Commit
   `feat(mod-93): /htui status, waiting and search`.
3. `tests/render.test.tsx` (red), then the band and pane hooks, to green. Then
   `claude plugin validate --strict crates/htui/claude-plugin/htui`. The `calls:` line must be
   exactly `$.clock.every, $.clock.now, $.command.register, $.env.get, $.process.run, $.state.get, $.state.set, $.ui.open, $.ui.panes, $.ui.resolve`,
   with `env reads: HTUI_SESSION`, `env writes: nothing`, and no gating hook. Commit
   `feat(mod-93): the AbovePrompt band and the /htui pane`.

---

## T3: `htui claude emit` and the gate

### `cli.rs`

- `Command::Claude(ClaudeArgs)`, placed after `Waiting`. Doc: "Write htui's Claude Code plugin
  (MOD-93). `htui claude emit` writes a directory marketplace and prints the two `claude plugin`
  commands. It runs neither, and never edits `~/.claude`."
- `pub struct ClaudeArgs { #[command(subcommand)] pub command: ClaudeCommand }`. It derives
  `Debug, Clone, PartialEq, Eq, clap::Args`. The subcommand is required, so `htui claude` alone is a
  usage error (exit 2).
- `pub enum ClaudeCommand { Emit(EmitArgs) }` (`clap::Subcommand`). Doc: what is written, the
  default dir, and that `--force` never replaces a directory `emit` did not write.
- `pub struct EmitArgs { #[arg(long, value_name = "PATH")] pub dir: Option<PathBuf>, #[arg(long)] pub force: bool }`.
  It derives `Default` too. Flag docs:
  - `dir`: "Write here instead of `<local data dir>/htui/claude-plugin`".
  - `force`: "Replace a directory an earlier `htui claude emit` wrote. A directory holding anything else is refused".

  No `env`.
- Tests, appended to `mod tests`:
  - `claude_emit_parses_its_two_flags`: bare; `--dir x --force`; `--force` alone.
  - `claude_needs_its_verb_and_refuses_tui_flags`: `htui claude` errors; `htui --demo claude emit` errors; `htui claude emit --decisions` errors.
  - `log_goes_after_claude_emit`: `htui claude emit --log x`.
  - `no_claude_emit_argument_reads_the_environment`: the `no_state_argument_reads_the_environment` walk on `find_subcommand("claude")?.find_subcommand("emit")`.
  - `the_version_line_is_htui_and_the_crate_version`: `Args::command().render_version() == format!("htui {}\n", env!("CARGO_PKG_VERSION"))`. This is the line D14 parses.

### `claude_emit.rs`: items (all `pub` items documented; `#![warn(missing_docs)]`, `missing_debug_implementations`)

Module doc in the `state_cmd.rs` style: "`htui claude emit` (MOD-93, `R-ID-4`, `R-ID-6`,
`R-STO-1`)". It says what is written, that the plugin's version is the binary's, the D4 ownership
rule, and that nothing outside the target and its two `.emit-<pid>`/`.old-<pid>` siblings is written
(parents excepted: `create_dir_all` of the target's parent). Exits are 2 (refused) and 1 (failed),
and none reaches Sentry.

```rust
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The Claude Code version mods need (ANA-29 §2.1); below it the skill alone is written (D6).
pub const CLAUDE_FLOOR: &str = "2.1.287";
/// `claude --version` prints `2.1.296 (Claude Code)`.
pub const CLAUDE_VERSION_PATTERN: &str = r"^(\d+\.\d+\.\d+)";
const SOURCE_VERSION: &str = "0.0.0-source";
const VERSION_TS_LITERAL: &str = "'0.0.0-source'";

#[derive(Debug)] pub enum EmitExit { Refused(String), Failed(String) }   // code(): 2 / 1; Display = sentence; Error
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub enum Variant { WithMod, SkillOnly }
#[derive(Debug, Clone, PartialEq, Eq)] pub enum ClaudeCheck { Met(String), Below(String), Unchecked }
impl ClaudeCheck {
    pub fn from_version(found: Option<String>) -> Self;   // below_min(v, CLAUDE_FLOOR) → Below, else Met; None → Unchecked
    pub const fn variant(&self) -> Variant;                // Below → SkillOnly; Met | Unchecked → WithMod
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)] pub struct Options { pub force: bool, pub variant: Variant }
#[derive(Debug, Clone, PartialEq, Eq)] pub struct Report {
    pub dir: PathBuf,              // absolute (std::path::absolute), what the printed command names
    pub variant: Variant,
    pub replaced: bool,            // an earlier emit was replaced (--force)
    pub leftover: Option<PathBuf>, // the old copy, when removing it failed after a successful swap
}

#[derive(Debug, Clone, Copy)] struct Embedded { path: &'static str, body: &'static str, part: Part }
#[derive(Debug, Clone, Copy, PartialEq, Eq)] enum Part { Always, Mod }
/// Every file of `crates/htui/claude-plugin/` but `htui/tests/` (D9) — D1's explicit table.
const FILES: [Embedded; 7] = [ /* the seven paths of "Coupling" 1; body: include_str!("../claude-plugin/<path>"); hooks/* and types/* are Part::Mod */ ];

pub fn default_dir() -> Option<PathBuf>;   // dirs::data_local_dir().map(|d| d.join("htui").join("claude-plugin")) — probe.rs:205's shape
pub async fn probe_claude() -> ClaudeCheck;
pub fn write_to(dir: &Path, options: &Options) -> Result<Report, EmitExit>;
pub fn render(report: &Report, check: &ClaudeCheck) -> String;
pub async fn run(args: cli::EmitArgs) -> Result<(), EmitExit>;
```

- **`probe_claude`**:
  ```rust
  ClaudeCheck::from_version(htui_agent::probe::capture_version(
      Path::new("claude"),
      &htui_agent::VersionProbe { args: vec!["--version".into()], pattern: CLAUDE_VERSION_PATTERN.into(), min: None },
      &htui_agent::ProbeEnv::host(std::env::temp_dir()),
  ).await)
  ```
  `ProbeEnv::host(cwd)` (`probe.rs:99`) gives `versions: true` and `VERSION_TIMEOUT` (15 s, a cold
  `node` allowance). The cwd is the temp dir, not the user's repo. The child inherits the process
  environment, because `launch.env` is empty and `spawn_supervised` only adds it (`launch.rs:1186`).
  On a timeout the child's tree is killed (`run_bounded`).
- **Stamping (D5).** Two mechanisms, chosen on purpose:
  - `plugin.json` is rewritten with `serde_json`. Parse it as `Value::Object`, set `"version"` to
    `VERSION`, and for `SkillOnly` remove `"types"` and `"userConfig"`. Then write
    `to_string_pretty` plus `\n`. The skill-only variant needs those keys removed anyway. One parse
    serves both variants, cannot emit invalid JSON, and fails loudly (`Failed`) if the source ever
    stops being an object. Key order changes (sorted); both orders validate (probed).
  - `version.ts` is code, not data, so it gets `body.replacen(VERSION_TS_LITERAL, &format!("'{VERSION}'"), 1)`.
    The unit test pins that the source holds the literal exactly once.
- **`write_to` sequence**:
  1. `dir = std::path::absolute(dir)`. If it fails, `Failed`.
  2. If there is no `file_name()` or no `parent()`, `Refused("<dir> names no directory to write")`.
  3. `classify(&dir)` uses `symlink_metadata`, never following a link. The cases:
     - **Absent**.
     - **Empty dir**.
     - **Ours**: the entries are a subset of `{".claude-plugin", "htui"}` and
       `.claude-plugin/marketplace.json` parses as an object with `"name": "htui"`.
     - **Foreign(why)**: the first foreign entry, "no marketplace.json", "marketplace.json is not
       htui's", or "a symbolic link".
     - **NotADirectory**.
  4. What each case does:
     - Foreign: `Refused("<dir> holds files htui claude emit did not write (<why>); it is left as it is. Choose another --dir.")`, with or without `--force`.
     - NotADirectory: `Refused("<dir> is a file, not a directory")`.
     - Ours without `force`: `Refused("<dir> already holds the htui plugin; pass --force to replace it")` (PRD: the refusal names the flag).
  5. `create_dir_all(parent)`. If it fails, `Failed`.
  6. `staging = parent/<name>.emit-<pid>`, made with `create_dir`, not `_all`, so an existing path
     fails. If it fails, `Failed("cannot create <staging>: <err>")` and nothing is touched.
  7. Write every `FILES` entry where `part == Always || variant == WithMod` into staging. Paths are
     split on `/` and joined, for Windows. Each parent gets `create_dir_all`, then `fs::write`. On
     any error: `remove_dir_all(staging)` (best effort), then `Failed`.
  8. Install:
     - **Absent**: `rename(staging, dir)`.
     - **Empty**: `remove_dir(dir)` (fails unless still empty), then `rename`.
     - **Ours with force**: `old = parent/<name>.old-<pid>`.
       1. `rename(dir, old)`. If it fails, clean up staging and return `Failed`.
       2. `rename(staging, dir)`. If it fails, `rename(old, dir)` puts the old copy back, staging is
          cleaned up, and the result is `Failed`. If that rollback also fails, the `Failed` sentence
          names `old` ("the previous plugin is at …").
       3. `remove_dir_all(old)`. If it fails, `leftover = Some(old)`. This still counts as success,
          and `render` prints it.

     Any other install error cleans up staging and returns `Failed`.
  9. `Ok(Report)`.

  The check-then-rename window (TOCTOU) is accepted: it is the user's own directory, and the
  check is a guard against mistakes, not against an attacker.
- **`render`** prints, each on its own line:
  1. `Wrote htui's Claude Code plugin <VERSION> to <dir>.`
  2. One of:
     - Met: `The mod is included (claude <v>).`
     - Below: `The mod is left out: claude <v> is below 2.1.287, so only the skill was written.`
     - Unchecked: `The mod is included; claude's version could not be checked (no answer from claude --version).`
  3. `Install it once:`
  4. `  claude plugin marketplace add <shell_words::quote(dir)>`
  5. `  claude plugin install htui@htui`
  6. `After upgrading htui: htui claude emit --force, then /reload-plugins in Claude Code.`
  7. With a leftover: `The old copy could not be removed: <path>.`

  `shell-words` is already a dependency.
- **`run(args)`**:
  1. `dir = args.dir.or_else(default_dir)`. If there is neither, `Refused("this machine has no local data directory; pass --dir PATH")`.
  2. `check = probe_claude().await`.
  3. `report = write_to(&dir, &Options { force: args.force, variant: check.variant() })?`.
  4. `print!("{}", render(&report, &check))`. Stdout errors are ignored except a non-`BrokenPipe`
     one, which gives `Failed`. `write_to` is sync `std::fs` (a few small files in a one-shot verb),
     and no `spawn_blocking` is needed.

### `lib.rs`, `main.rs`, `Cargo.toml`

- `lib.rs`: add `pub mod claude_emit;` in alphabetical order. In `run`, right after the `Waiting`
  branch:
  `if let Some(cli::Command::Claude(cli::ClaudeArgs { command: cli::ClaudeCommand::Emit(emit) })) = args.command.clone() { return claude_emit::run(emit).await.map_err(anyhow::Error::from); }`.
  It goes after `init_tracing`, so `--log` works. Extend `run`'s "Startup order" doc by one sentence.
- `main.rs`:
  - `exit_code` gets one more `.or_else(|| error.downcast_ref::<htui::claude_emit::EmitExit>().map(htui::claude_emit::EmitExit::code))`.
  - `reports_to_sentry` gets one more `&& error.downcast_ref::<htui::claude_emit::EmitExit>().is_none()`,
    and its doc gets "MOD-93: no `htui claude emit` end, whatever its code: its sentences carry home
    paths".
  - `main`'s exit-code doc lists the emit refusal under 2.
  - New test `emit_exits_are_2_1_and_never_reach_sentry`, in the shape of
    `state_exits_are_3_2_1_and_only_failures_reach_sentry`, wrapped and unwrapped.
- `crates/htui/Cargo.toml`: `dirs = { workspace = true }`, with a comment ("MOD-93 D3: the default
  emit dir; 6.0.0 is already in the lock through htui-store"). `Cargo.lock` gains `"dirs"` in
  `htui`'s dependency list only.

### Unit tests in `claude_emit.rs` (TDD order; `tempfile::tempdir()`, which is a normal dependency)

1. `emit_exits_are_2_and_1`.
2. `every_source_file_is_embedded_and_every_embedded_file_is_the_source`. A recursive
   `std::fs::read_dir` walk of `concat!(env!("CARGO_MANIFEST_DIR"), "/claude-plugin")` includes
   dot-directories and leaves out `htui/tests/` and `htui/.claude-plugin/types/`. The set of
   relative `/`-paths must equal the `FILES` paths, and each `FILES.body` must equal the file read
   from disk (which proves each `include_str!` points at its own path).
3. `the_source_carries_the_placeholder_version_once`. `plugin.json` has `version ==
   SOURCE_VERSION`, and `version.ts` has `matches(VERSION_TS_LITERAL).count() == 1`.
4. `the_stamped_version_is_the_binarys_in_both_places`. Emit `WithMod`; the parsed `plugin.json`
   version is `VERSION`, and `version.ts == format!("export const VERSION = '{VERSION}'\n")`.
5. `the_full_variant_keeps_hooks_types_and_user_config`.
6. `the_skill_only_variant_has_no_hooks_types_or_their_manifest_keys`.
7. `an_absent_directory_is_created_with_its_parents`.
8. `an_empty_directory_is_filled_without_force`.
9. `a_non_empty_directory_is_refused_without_force_and_the_refusal_names_the_flag`.
10. `force_replaces_an_earlier_emit_and_leaves_no_stale_file`. Emit, plant
    `htui/hooks/stale.ts`, then `--force` `SkillOnly`: `stale.ts` and `hooks/` are gone.
11. `a_foreign_directory_is_refused_even_with_force_and_left_untouched`. Four cases:
    - an extra `notes.txt`;
    - `marketplace.json` named `other`;
    - no `marketplace.json`;
    - `marketplace.json` that is not JSON.

    Each must be `Refused`, with a byte-identical tree before and after (a map of path to bytes).
12. `a_file_target_is_refused`, plus `#[cfg(unix)] a_symlinked_target_is_refused`.
13. `nothing_is_left_beside_the_target`. After each of 7, 8 and 10, the parent tempdir lists only
    the target.
14. `a_staging_failure_leaves_the_target_as_it_was`. Pre-create `<name>.emit-<std::process::id()>`
    as a file: `Failed`, and an earlier emit's tree is unchanged.
15. `the_floor_decides_the_variant`. `2.1.286` gives `Below`/`SkillOnly`; `2.1.287` and `2.1.296`
    give `Met`/`WithMod`; `None` gives `Unchecked`/`WithMod`. Also
    `extract_version("2.1.296 (Claude Code)", CLAUDE_VERSION_PATTERN) == Some("2.1.296")`.
16. `the_report_prints_both_commands_and_the_rerun_hint`, for the three checks, a path with a
    space (quoted), and a leftover.
17. `the_default_dir_is_under_the_local_data_dir`.

### Gate test `crates/htui/tests/claude_plugin.rs` (D10; no `cfg(feature)`)

One `#[tokio::test] async fn the_emitted_plugin_validates_and_the_mod_tests_pass_under_claude()`:
1. `match htui::claude_emit::probe_claude().await`:
   - `Unchecked`: `eprintln!("skip: no claude on PATH answered --version; MOD-93's plugin gate did not run")` and return.
   - `Below(v)`: the same, with `claude {v} is below {CLAUDE_FLOOR}`.
   - `Met(_)`: go on.

   This is the production probe, so the gate also exercises it.
2. `config = tempdir()`, `out = tempdir()`. `write_to(out/full, WithMod)` and
   `write_to(out/skill, SkillOnly)`. Assert `out/full/htui/tests` does not exist (D9).
3. A helper `claude(args, config) -> (ExitStatus, String)` uses
   `tokio::process::Command::new("claude")` with:
   - `.args(args)`
   - `.env("CLAUDE_CONFIG_DIR", config)`
   - `.env_remove("CLAUDE_CODE_PLUGIN_DIRS")`
   - `.current_dir(out)`
   - `.stdin(Stdio::null())`
   - `.kill_on_drop(true)`
   - `tokio::time::timeout(120 s)`

   A timeout panics with the command line: a failure, not a skip.
4. `validate --strict --json <p>` for `p` in each of:
   - source `claude-plugin`
   - source `claude-plugin/htui`
   - `out/full`
   - `out/full/htui`
   - `out/skill`
   - `out/skill/htui`

   Each must exit 0 with `serde_json` `success == true`. Validating the source tree also catches a
   T1/T2 edit that breaks the manifest.
5. For `out/full/htui`'s JSON:
   - every `contents[].gatingHooks` is empty;
   - the note starting `./register.tsx calls: ` lists only names in
     `{$.clock.every, $.clock.now, $.command.register, $.env.get, $.process.run, $.state.get, $.state.set, $.ui.open, $.ui.panes, $.ui.resolve}`;
   - the env-reads note is `HTUI_SESSION`;
   - the env-writes note is `nothing`.

   This is ANA-29 §6.3: "the `calls:` line is the check".
6. `plugin test <source>/claude-plugin/htui` exits 0.

Commands run only on temp dirs and the source tree. Nothing writes into the source tree (probed:
`validate`, `test` and `install` add no files).

### T3 TDD order and commits

1. Cargo `dirs`, a `claude_emit.rs` skeleton (`EmitExit`, test 1), and `main.rs` mapping plus its
   test. Commit `feat(mod-93): EmitExit and its exit mapping`. Stage `crates/htui/Cargo.toml`,
   `Cargo.lock`, `crates/htui/src/claude_emit.rs`, `crates/htui/src/main.rs` and
   `crates/htui/src/lib.rs` (the `pub mod` line only).
2. Tests 2-6 (red), then `FILES` and stamping. Commit `feat(mod-93): the embedded plugin table and version stamping`.
3. Tests 7-14 (red), then `classify` and `write_to`. Commit `feat(mod-93): emit's target rules and the --force swap`.
4. The cli tests (red), then `cli.rs`, `probe_claude`/`render`/`run` with tests 15-17, and the
   `lib.rs` dispatch. Commit `feat(mod-93): htui claude emit`.
5. `tests/claude_plugin.rs`. Commit `test(mod-93): the claude plugin gate`.

Run, in this order:
1. `cargo test -p htui --lib claude_emit`
2. `cargo test -p htui --lib cli::tests`
3. `cargo test -p htui --bin htui`
4. `cargo test -p htui --test claude_plugin -- --nocapture` (check that it did not print `skip:`)
5. `cargo clippy -p htui --all-targets --all-features -- -D warnings`
6. `cargo clippy --workspace -- -D warnings`

---

## T4: docs

`docs/htui-claude-plugin.md` follows the plan's T4 outline, in `docs/htui-cli.md`'s voice. Every
flag, default, refusal sentence and exit code comes from §T3. The band, pane and command texts come
from §T2 "Text formats". `docs/htui-cli.md` gets one "See also: [the Claude Code
plugin](htui-claude-plugin.md)" line after the intro list. `README.md` gets a short "Use htui from
Claude Code" section: `htui claude emit`, the two commands, and a link to the doc. Commit
`docs(mod-93): the htui Claude Code plugin`, staging the three paths by name.

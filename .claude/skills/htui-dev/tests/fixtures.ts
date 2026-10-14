// Shared helpers for the mod's kit tests (`claude plugin test`). The tests stand
// for the engine beneath the mod: what a hook here answers reaches the mod as given.
import type { On } from 'claude-code'
import type { Engine, MockClock } from 'claude-code/testing'
import { mock } from 'claude-code/testing'

/** The literal crates/htui-agent/tests/extensibility.rs forbids, never spelled whole in this repo. */
export const LIT = 'ze' + 'ta'

/** One tool call that got past the mod: the tool, and the agentId it reached the engine with. */
export type Reached = { tool: string; agentId: string | undefined }

/** Stands in for the tools beneath the mod: every call that gets through answers `ok`, recorded in `reached`. */
export const tools = (on: On, reached: Reached[]) => {
  on('tool.call', async ($, e) => {
    reached.push({ tool: e.tool, agentId: (e as { agentId?: string }).agentId })
    return { result: 'ok' } as never
  })
}

/** One spawn that got past the mod: its type, its model, and the parent it reached the engine with. */
export type Spawned = { subagentType: string; model: string | undefined; parentAgentId: string | undefined }

/**
 * Stands in for the engine's spawn beneath the mod: each spawn that gets through starts
 * `agent-<n>`, counting from 1, on `model` (or `claude-opus-5-5`); recorded in `spawned`.
 * `answer` replaces the answer, for a spawn that starts no agent or is refused below.
 */
export const agents = (on: On, spawned: Spawned[], answer?: (n: number) => object) => {
  on('agent.spawn', async ($, e) => {
    spawned.push({ subagentType: e.subagentType, model: e.model, parentAgentId: e.parentAgentId })
    const n = spawned.length
    return (answer ? answer(n) : { model: e.model ?? 'claude-opus-5-5', agentId: `agent-${n}` }) as never
  })
}

/** Records every `$.ui.status` line the mod sets; the kit has no bottom for it. */
export const statuses = (on: On, lines: (string | undefined)[]) => {
  on('ui.status', async ($, e) => {
    lines.push(e.text)
    return { value: undefined } as never
  })
}

/** Records every value the mod writes to its `readOnly` tags. */
export const tagWrites = (on: On, writes: unknown[]) => {
  on('state.set', { plugin: 'htui-dev', key: 'readOnly' }, async ($, e, next) => {
    writes.push(e.value)
    return next(e)
  })
}

/** Answers every read of the `readOnly` tags with `value`, as after a reload. Never tag in a test that seeds. */
export const seedTags = (on: On, value: unknown) => {
  on('state.get', { plugin: 'htui-dev', key: 'readOnly' }, async () => ({ value: { value, version: 1 } }) as never)
}

/** Runs `command` through Bash, from the main loop or from the subagent `agentId`. */
export const bash = ($: Engine, command: string, agentId?: string) =>
  $.tool.call({ tool: 'Bash', command, ...(agentId === undefined ? {} : { agentId }) } as never)

/** Calls `tool` with `input`, from the main loop or from the subagent `agentId`. */
export const call = ($: Engine, tool: string, input: object, agentId?: string) =>
  $.tool.call({ tool, ...input, ...(agentId === undefined ? {} : { agentId }) } as never)

/** Spawns a subagent as the Agent tool would; `parentAgentId` makes it a nested spawn. */
export const spawn = (
  $: Engine,
  args: { prompt?: string; subagentType?: string; model?: string; parentAgentId?: string },
) => $.agent.spawn({ prompt: 'do the task', subagentType: 'general-purpose', ...args } as never)

// ---- M2: the status line and the commands ----

/** The checkout `git rev-parse --show-toplevel` names in the M2 tests, and its HEAD. */
export const ROOT = '/repo'
export const HEAD = 'a'.repeat(40)

/** A `process.run` answer: the child exited with `exitCode` after writing `stdout` and `stderr`. */
export const ok = (stdout: string, exitCode = 0, stderr = '') => ({
  value: { exitCode, stdout, stderr, isStdoutTruncated: false, isStderrTruncated: false },
})

/** How the fake host answers one run: a finished run, a spawn failure, or a run that waits `hang` ms on the mocked clock and then fails. */
export type Answer = ReturnType<typeof ok> | 'deny' | { hang: number }

/** One `$.process.run` the mod made. */
export type Run = { argv: string[]; cwd?: string; timeoutMs?: number }

/** What each run the mod makes answers; a run left out is a spawn failure. */
export type World = Partial<Record<'root' | 'status' | 'merge' | 'revList' | 'head' | 'health' | 'next' | 'validate' | 'gate', Answer>>

/** Which run an argv is: a git verb, or a script by its file name (bash's argv[1], pwsh's argv[3]). */
export const keyOf = (argv: readonly string[]): string => {
  if (argv[0] === 'git') {
    if (argv.includes('--show-toplevel')) return 'root'
    if (argv.includes('status')) return 'status'
    if (argv.includes('MERGE_HEAD')) return 'merge'
    if (argv.includes('rev-list')) return 'revList'
    if (argv.includes('rev-parse')) return 'head'
    return 'git'
  }
  const script = (argv[0] === 'pwsh' ? argv[3] : argv[1]) ?? ''
  const base = script.replace(/^.*\//, '').replace(/\.(sh|ps1)$/, '')
  return ({ 'dev-health': 'health', 'next-item': 'next', 'validate-workflow-docs': 'validate', gate: 'gate' } as Record<string, string>)[base] ?? base
}

/** `git status --porcelain=v2 --branch` output. */
export const porcelain = (o: { head?: string | null; ab?: [number, number]; entries?: string[] } = {}) =>
  [
    `# branch.oid ${HEAD}`,
    `# branch.head ${o.head === null ? '(detached)' : (o.head ?? 'hr/TOOL-8')}`,
    ...(o.ab ? ['# branch.upstream origin/x', `# branch.ab +${o.ab[0]} -${o.ab[1]}`] : []),
    ...(o.entries ?? []),
    '',
  ].join('\n')

/** scripts/dev-health's JSON line. */
export const health = (usedPct: number | null = 80, state = 'up') =>
  JSON.stringify({ disk: { path: ROOT, used_pct: usedPct }, postgres: { state, via: 'pg_isready', target: 'localhost:5439' } })

/** A checkout on hr/TOOL-8, 2 ahead of its upstream, 1 file dirty, no merge, disk 80%, Postgres up. */
export const DEFAULT_WORLD: World = {
  root: ok(`${ROOT}\n`),
  status: ok(porcelain({ ab: [2, 0], entries: ['? new.txt'] })),
  merge: ok('', 1),
  revList: ok('0\n'),
  head: ok(`${HEAD}\n`),
  health: ok(health()),
}

/** Stands in for the host's processes: answers each run from `w` (read at call time, so a test may change it), recorded in `runs`. */
export const world = (on: On, w: World, runs: Run[], clock?: MockClock) => {
  on('process.run', async ($, e) => {
    runs.push({ argv: [...e.argv], cwd: e.init?.cwd, timeoutMs: e.init?.timeoutMs })
    const answer = (w as Record<string, Answer | undefined>)[keyOf(e.argv)] ?? 'deny'
    if (answer === 'deny') return { deny: 'spawn ENOENT' } as never
    if ('hang' in answer) {
      await clock!.sleep(answer.hang)
      return { deny: 'timed out' } as never
    }
    return answer as never
  })
}

/** The runs among `runs` that `keyOf` names `key`. */
export const countRuns = (runs: Run[], key: string) => runs.filter(r => keyOf(r.argv) === key).length

/** A HANDOFF.md whose TOOL-8 has landed M1. */
export const HANDOFF = [
  '### Tooling findings',
  '',
  '- [ ] **TOOL-8 - `htui-dev` mod.** Body. **Phase M1 landed (`1d3963c3`, 2026-10-14):** guards.',
  '- [ ] **TOOL-10 - next item.** body',
  '',
].join('\n')

/** Stands in for the files the mod reads: `map[path]` (read at call time), else a missing file; each path read is recorded. */
export const files = (on: On, map: Record<string, string>, reads: string[] = []) => {
  on('fs.read', async ($, e) => {
    const path = (e as unknown as { path: string }).path
    reads.push(path)
    return path in map ? ({ value: map[path] } as never) : ({ deny: `ENOENT: ${path}` } as never)
  })
}

/** Records every `$.ui.toast` text. */
export const toasts = (on: On, list: string[]) => {
  on('ui.toast', async ($, e) => {
    list.push((e as unknown as { text: string }).text)
    return { value: undefined } as never
  })
}

/**
 * Raises `session.start` as a session does: `env` is the whole process environment (HR_ITEM=TOOL-8
 * unless given), `isInteractive` true unless given. Records each registered command in `names`.
 */
export const start = async ($: Engine, on: On, o: { env?: Record<string, string>; isInteractive?: boolean; names?: string[] } = {}) => {
  mock.env(on, o.env ?? { HR_ITEM: 'TOOL-8' })
  on('command.register', async ($, e) => {
    o.names?.push((e as unknown as { name: string }).name)
    return { value: { command: (e as unknown as { name: string }).name } } as never
  })
  on('session.start', async ($, e) => e)
  const isInteractive = o.isInteractive ?? true
  await $.session.start({ cwd: ROOT, surface: isInteractive ? 'terminal' : null, isInteractive } as never)
}

/** Runs the slash command `/name args` as the person would. */
export const command = ($: Engine, name: 'gate' | 'next' | 'validate', args = '') =>
  $.command.run({ command: name, args } as never) as Promise<{ text?: string; exitCode?: number }>

/** Answers every read of `ambient` with `value`, as after a reload; counts the reads. */
export const seedAmbient = (on: On, value: boolean, reads: { n: number }) => {
  on('state.get', { plugin: 'htui-dev', key: 'ambient' }, async () => {
    reads.n++
    return { value: { value, version: 1 } } as never
  })
}

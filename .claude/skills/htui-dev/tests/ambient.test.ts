import type { On } from 'claude-code'
import type { Engine } from 'claude-code/testing'
import { test, expect, mock } from 'claude-code/testing'
import {
  bash,
  command,
  countRuns,
  DEFAULT_WORLD,
  files,
  HANDOFF,
  HEAD,
  health,
  keyOf,
  ok,
  porcelain,
  ROOT,
  seedAmbient,
  start,
  statuses,
  toasts,
  tools,
  world,
  type Reached,
  type Run,
  type World,
} from './fixtures'

const LAST = `${ROOT}/target/gate/last.json`
const BASE_FILES = { [`${ROOT}/HANDOFF.md`]: HANDOFF }
const LINE = 'TOOL-8 M1 landed · 2 ahead · 1 dirty'

/** `last.json` with `started`/`finished` as ISO times (finished null while running). */
const gateJson = (state: 'running' | 'passed' | 'failed', started: string, finished: string | null, failed: string[] = []) =>
  JSON.stringify({
    head: HEAD,
    started,
    finished,
    state,
    steps: failed.map(name => ({ name, status: 'failed', seconds: 1 })),
  })

/** A session over DEFAULT_WORLD (plus `w`), its files and its status lines, started interactive unless told. */
const setup = async (
  $: Engine,
  on: On,
  o: { w?: World; files?: Record<string, string>; env?: Record<string, string>; isInteractive?: boolean } = {},
) => {
  const clock = mock.clock(on, { now: Date.parse('2026-10-14T12:00:00Z') })
  const runs: Run[] = []
  const lines: (string | undefined)[] = []
  const shown: string[] = []
  const names: string[] = []
  const w: World = { ...DEFAULT_WORLD, ...o.w }
  const fs: Record<string, string> = { ...BASE_FILES, ...o.files }
  world(on, w, runs, clock)
  files(on, fs)
  statuses(on, lines)
  toasts(on, shown)
  await start($, on, { env: o.env, isInteractive: o.isInteractive, names })
  await clock.settle()
  return { clock, runs, lines, shown, names, w, fs }
}

test('an interactive session shows the line at start', async ($, on) => {
  const { lines, runs } = await setup($, on)
  expect(lines).toEqual([LINE])
  const status = runs.find(r => keyOf(r.argv) === 'status')!
  expect(status.argv).toEqual(['git', '--no-optional-locks', 'status', '--porcelain=v2', '--branch'])
  expect(status.cwd).toBe(ROOT)
  expect(runs.find(r => keyOf(r.argv) === 'health')!.argv).toEqual(['bash', `${ROOT}/scripts/dev-health.sh`])
})

test('the line refreshes every 60 s, and is re-sent only when it changed', async ($, on) => {
  const { clock, runs, lines, w } = await setup($, on)
  await clock.advance(59_999)
  expect(countRuns(runs, 'status')).toBe(1)
  await clock.advance(1)
  expect(countRuns(runs, 'status')).toBe(2)
  expect(countRuns(runs, 'health')).toBe(2)
  expect(lines).toEqual([LINE])
  w.status = ok(porcelain({ ab: [3, 0] }))
  await clock.advance(60_000)
  expect(lines).toEqual([LINE, 'TOOL-8 M1 landed · 3 ahead'])
})

test('a -p session arms no timer and runs nothing', async ($, on) => {
  const { clock, runs, lines, names } = await setup($, on, { isInteractive: false })
  await clock.advance(120_000)
  expect(runs).toEqual([])
  expect(lines).toEqual([])
  expect(names).toEqual(['gate', 'next', 'validate'])
})

test('a session htui drives arms no timer', async ($, on) => {
  const { clock, runs, names } = await setup($, on, { env: { HR_ITEM: 'TOOL-8', HTUI_SESSION: '1' } })
  await clock.advance(120_000)
  expect(runs).toEqual([])
  expect(names).toEqual(['gate', 'next', 'validate'])
})

test('status_line off arms no timer', { options: { status_line: false } }, async ($, on) => {
  const { clock, runs, lines, names } = await setup($, on)
  await clock.advance(120_000)
  expect(runs).toEqual([])
  expect(lines).toEqual([])
  expect(names).toEqual(['gate', 'next', 'validate'])
})

test('a slow tick is never overlapped', async ($, on) => {
  const { clock, runs } = await setup($, on, { w: { health: { hang: 90_000 } } })
  expect(countRuns(runs, 'health')).toBe(1)
  await clock.advance(60_000)
  expect(countRuns(runs, 'health')).toBe(1)
  await clock.advance(30_000)
  await clock.advance(30_000)
  expect(countRuns(runs, 'health')).toBe(2)
})

test('merging, a full disk and a down Postgres show; a quiet disk and an up Postgres do not', async ($, on) => {
  const { lines } = await setup($, on, { w: { merge: ok(`${HEAD}\n`), health: ok(health(93, 'down')) } })
  expect(lines).toEqual(['TOOL-8 M1 landed · merging · 2 ahead · 1 dirty · disk 93% · pg down'])
})

test('a dev-health that cannot run or prints bad JSON leaves the rest of the line', async ($, on) => {
  const { lines, clock, w } = await setup($, on, { w: { health: 'deny' } })
  expect(lines).toEqual([LINE])
  w.health = ok('not json')
  await clock.advance(60_000)
  expect(lines).toEqual([LINE])
})

test('no upstream off main counts commits ahead of main', async ($, on) => {
  const { lines, runs } = await setup($, on, { w: { status: ok(porcelain()), revList: ok('4\n') } })
  expect(lines).toEqual(['TOOL-8 M1 landed · 4 ahead'])
  expect(runs.find(r => keyOf(r.argv) === 'revList')!.argv).toEqual(['git', 'rev-list', '--count', 'main..HEAD'])
})

test('with no HR_ITEM the item comes from the branch', async ($, on) => {
  const { lines } = await setup($, on, { env: {}, w: { status: ok(porcelain({ head: 'hr/tool-10' })) } })
  expect(lines).toEqual(['TOOL-10 open'])
})

test('an unreadable HANDOFF shows the item without a phase', async ($, on) => {
  const { lines } = await setup($, on, { files: { [`${ROOT}/HANDOFF.md`]: undefined as never } })
  expect(lines).toEqual(['TOOL-8 · 2 ahead · 1 dirty'])
})

test('an unresolvable repo root says so on the line', async ($, on) => {
  const { lines } = await setup($, on, { w: { root: 'deny' } })
  expect(lines).toEqual(['htui-dev: no repo root (git rev-parse failed)'])
})

test('the last gate result is on the line', async ($, on) => {
  const { lines } = await setup($, on, {
    files: { [LAST]: gateJson('failed', '2026-10-14T10:00:00Z', '2026-10-14T11:20:00Z', ['test']) },
  })
  expect(lines).toEqual([`${LINE} · gate failed: test 40m`])
})

test('a gate that finished before the session never toasts', async ($, on) => {
  const { clock, shown } = await setup($, on, { files: { [LAST]: gateJson('passed', '2026-10-14T10:00:00Z', '2026-10-14T11:00:00Z') } })
  await clock.advance(120_000)
  expect(shown).toEqual([])
})

test('a gate that finishes during the session toasts once', async ($, on) => {
  const { clock, shown, fs } = await setup($, on, { files: { [LAST]: gateJson('passed', '2026-10-14T10:00:00Z', '2026-10-14T11:00:00Z') } })
  fs[LAST] = gateJson('failed', '2026-10-14T12:00:10Z', '2026-10-14T12:00:50Z', ['test'])
  await clock.advance(60_000)
  expect(shown).toEqual(['gate failed: test'])
  await clock.advance(120_000)
  expect(shown).toEqual(['gate failed: test'])
})

test('a gate running at start toasts when it finishes', async ($, on) => {
  const { clock, shown, fs } = await setup($, on, { files: { [LAST]: gateJson('running', '2026-10-14T11:50:00Z', null) } })
  expect(shown).toEqual([])
  fs[LAST] = gateJson('passed', '2026-10-14T11:50:00Z', '2026-10-14T12:00:30Z')
  await clock.advance(60_000)
  expect(shown).toEqual(['gate passed'])
})

test('a guard warning leads the line, then drops 10 minutes after the tick that first showed it', async ($, on) => {
  const reached: Reached[] = []
  tools(on, reached)
  // A read of the read-only tags that fails is a guard failure: the call goes through, with a warning.
  on('state.get', { plugin: 'htui-dev', key: 'readOnly' }, async () => ({ deny: 'store down' }) as never)
  const { clock, lines } = await setup($, on)
  await bash($, 'ls', 'agent-1')
  expect(reached.length).toBe(1)
  const warned = `⚠ htui-dev: read-only guard failed, call allowed · ${LINE}`
  expect(lines.at(-1)).toBe(warned)
  // The tick at 60 s stamps it; the one at 660 s, 10 minutes on, drops it.
  await clock.advance(60_000)
  await clock.advance(540_000)
  expect(lines.at(-1)).toBe(warned)
  await clock.advance(60_000)
  expect(lines.at(-1)).toBe(LINE)
})

test('after a reload, a tool call re-arms the timer once', async ($, on) => {
  const clock = mock.clock(on)
  const runs: Run[] = []
  const lines: (string | undefined)[] = []
  const reads = { n: 0 }
  world(on, { ...DEFAULT_WORLD }, runs, clock)
  files(on, BASE_FILES)
  statuses(on, lines)
  mock.env(on, { HR_ITEM: 'TOOL-8' })
  seedAmbient(on, true, reads)
  tools(on, [])
  await bash($, 'ls')
  await clock.settle()
  expect(lines).toEqual([LINE])
  await bash($, 'ls')
  await clock.settle()
  expect(reads.n).toBe(1)
  await clock.advance(60_000)
  expect(countRuns(runs, 'status')).toBe(2)
})

test("a subagent's tool call never re-arms the timer", async ($, on) => {
  const clock = mock.clock(on)
  const runs: Run[] = []
  const reads = { n: 0 }
  world(on, { ...DEFAULT_WORLD }, runs, clock)
  seedAmbient(on, true, reads)
  tools(on, [])
  await bash($, 'ls', 'agent-1')
  await clock.advance(120_000)
  expect(reads.n).toBe(0)
  expect(runs).toEqual([])
})

test('a state store that fails stops neither the line nor its refresh', async ($, on) => {
  on('state.set', { plugin: 'htui-dev', key: 'gateSeen' }, async () => ({ deny: 'store down' }) as never)
  const { clock, lines, w } = await setup($, on, {
    files: { [LAST]: gateJson('passed', '2026-10-14T10:00:00Z', '2026-10-14T11:00:00Z') },
  })
  expect(lines).toEqual([`${LINE} · gate passed 1h`])
  w.status = ok(porcelain({ ab: [3, 0] }))
  await clock.advance(60_000)
  expect(lines.at(-1)).toBe('TOOL-8 M1 landed · 3 ahead · gate passed 1h')
})

test('a gate that ended before a session whose root was not found at first never toasts', async ($, on) => {
  const { clock, shown, w } = await setup($, on, {
    w: { root: 'deny' },
    files: { [LAST]: gateJson('passed', '2026-10-14T10:00:00Z', '2026-10-14T11:00:00Z') },
  })
  w.root = ok(`${ROOT}\n`)
  await clock.advance(120_000)
  expect(shown).toEqual([])
})

test('a tool call re-arms nothing when no session armed a timer', async ($, on) => {
  const clock = mock.clock(on)
  const runs: Run[] = []
  world(on, { ...DEFAULT_WORLD }, runs, clock)
  seedAmbient(on, false, { n: 0 })
  tools(on, [])
  await bash($, 'ls')
  await clock.advance(120_000)
  expect(runs).toEqual([])
})

test('status_line off after a reload clears the line instead', { options: { status_line: false } }, async ($, on) => {
  const clock = mock.clock(on)
  const runs: Run[] = []
  const lines: (string | undefined)[] = []
  world(on, { ...DEFAULT_WORLD }, runs, clock)
  statuses(on, lines)
  seedAmbient(on, true, { n: 0 })
  tools(on, [])
  await bash($, 'ls')
  await clock.advance(120_000)
  expect(runs).toEqual([])
  expect(lines).toEqual([undefined])
})

test('a second session.start arms no second timer', async ($, on) => {
  const { clock, runs } = await setup($, on)
  await $.session.start({ cwd: ROOT, surface: 'terminal', isInteractive: true } as never)
  await clock.settle()
  const before = countRuns(runs, 'status')
  await clock.advance(60_000)
  expect(countRuns(runs, 'status')).toBe(before + 1)
})

// ---- the commands through the engine ----

test('/validate answers the validator and its exit code', async ($, on) => {
  const { runs } = await setup($, on, {
    isInteractive: false,
    w: { validate: ok('workflow-docs validation: 1 error(s), 0 warning(s)\n', 1) },
  })
  expect(await command($, 'validate')).toEqual({ text: 'workflow-docs validation: 1 error(s), 0 warning(s)', exitCode: 1 })
  const run = runs.find(r => keyOf(r.argv) === 'validate')!
  expect(run.argv).toEqual(['bash', `${ROOT}/.claude/skills/handoff-run/scripts/validate-workflow-docs.sh`, '--repo-root', ROOT])
  expect(run.cwd).toBe(ROOT)
})

test('/next answers the ranking', async ($, on) => {
  await setup($, on, { isInteractive: false, w: { next: ok('next-item pre-filter\n  1. TOOL-8\n') } })
  expect(await command($, 'next')).toEqual({ text: 'next-item pre-filter\n  1. TOOL-8', exitCode: 0 })
})

test('/gate starts the detached gate', async ($, on) => {
  const { runs } = await setup($, on, { isInteractive: false, w: { gate: ok('gate started, pid 7\n') } })
  expect(await command($, 'gate', 'fmt')).toEqual({ text: 'gate started, pid 7', exitCode: 0 })
  expect(runs.find(r => keyOf(r.argv) === 'gate')!.argv).toEqual(['bash', `${ROOT}/scripts/gate.sh`, '--detach', '--only', 'fmt'])
})

test('/gate status marks a result for another HEAD', async ($, on) => {
  const doc = JSON.stringify({ head: 'b'.repeat(40), started: '2026-10-14T11:00:00Z', finished: '2026-10-14T11:30:00Z', state: 'passed', steps: [] })
  await setup($, on, { isInteractive: false, files: { [LAST]: doc } })
  expect(await command($, 'gate', 'status')).toEqual({ text: 'gate passed 30m (older HEAD)', exitCode: 0 })
})

test('the repo root is found once, from the plugin folder', async ($, on) => {
  const { runs } = await setup($, on, { isInteractive: false, w: { next: ok('x\n'), validate: ok('y\n') } })
  await command($, 'next')
  await command($, 'validate')
  await command($, 'next')
  const roots = runs.filter(r => keyOf(r.argv) === 'root')
  expect(roots.length).toBe(1)
  expect(roots[0]!.argv).toEqual(['git', 'rev-parse', '--show-toplevel'])
  expect(roots[0]!.cwd).toMatch(/[\\/]\.claude[\\/]skills[\\/]htui-dev$/)
})

test('with no repo root each command says so and runs no script', async ($, on) => {
  const { runs } = await setup($, on, { isInteractive: false, w: { root: 'deny' } })
  for (const name of ['gate', 'next', 'validate'] as const) {
    expect(await command($, name)).toEqual({
      text: 'htui-dev: cannot find the repo root (git rev-parse --show-toplevel failed)',
      exitCode: 1,
    })
  }
  // Each command asks again (a failure is not kept), and runs nothing else.
  expect(runs.length).toBe(3)
  expect(runs.every(r => keyOf(r.argv) === 'root')).toBe(true)
})

test('on Windows the commands run the .ps1 through pwsh', async ($, on) => {
  const { runs } = await setup($, on, { isInteractive: false, env: { OS: 'Windows_NT' }, w: { next: ok('x\n') } })
  await command($, 'next')
  expect(runs.find(r => keyOf(r.argv) === 'next')!.argv).toEqual([
    'pwsh',
    '-NoProfile',
    '-File',
    `${ROOT}/.claude/skills/handoff-run/scripts/next-item.ps1`,
    '-RepoRoot',
    ROOT,
  ])
})

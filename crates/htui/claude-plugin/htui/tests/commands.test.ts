import { test, expect, mock } from 'claude-code/testing'
import type { On } from 'claude-code'
import type { Engine, MockClock } from 'claude-code/testing'
import {
  htui,
  NO_DSN,
  OFFLINE,
  ok,
  SEARCH,
  SEARCH_EMPTY,
  start,
  STATUS,
  WAITING,
  WAITING_NONE,
  type Answer,
  type Call,
} from './fixtures'

const SKEW = 'htui 9.9.9 ≠ plugin 0.0.0-source: run htui claude emit --force, then /reload-plugins'

/**
 * Starts a session (F7b: /htui runs only in one that did not stand down) and lets its first
 * `--version` and `waiting` runs settle, then forgets them from `calls`.
 */
const begin = async ($: Engine, on: On, clock: MockClock, calls?: Call[]) => {
  await start($, on)
  await clock.settle()
  if (calls) calls.length = 0
}

/** Runs `/htui <args>` and answers its text. */
const run = async ($: Engine, args: string) =>
  (await $.command.run({ command: 'htui', args } as never)).text

test('status prints the top bar\'s line', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { status: ok(STATUS) }, calls)
  await begin($, on, clock, calls)
  expect(await run($, 'status')).toBe('Platform · desk · online · 1 working · 2 waiting')
  expect(calls).toEqual([{ argv: ['htui', 'status', '--json'], timeoutMs: 10000 }])
})

test('status says when permission requests could not be listed', async ($, on) => {
  const clock = mock.clock(on)
  const doc = { ...JSON.parse(STATUS), workspace: null, box: null, permissions_known: false }
  htui(on, { status: ok(JSON.stringify(doc)) }, [])
  await begin($, on, clock)
  expect(await run($, 'status')).toBe(
    'no workspace · no box · online · 1 working · 2 waiting\npermission requests could not be listed; the count may be low',
  )
})

test('status shows the skew line', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { status: ok(STATUS), waiting: ok(WAITING), '--version': ok('htui 9.9.9\n') }, calls)
  await start($, on)
  await clock.settle()
  expect(calls).toContainEqual({ argv: ['htui', '--version'], timeoutMs: 10000 })
  expect(await run($, 'status')).toBe(`Platform · desk · online · 1 working · 2 waiting\n${SKEW}`)
})

test('a matching binary shows no skew line', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { status: ok(STATUS), waiting: ok(WAITING), '--version': ok('htui 0.0.0-source\n') }, [])
  await start($, on)
  await clock.settle()
  expect(await run($, 'status')).toBe('Platform · desk · online · 1 working · 2 waiting')
})

test('waiting prints one line per row, and a sentence when none', async ($, on) => {
  const clock = mock.clock(on)
  const answers: Record<string, Answer> = { waiting: ok(WAITING) }
  const calls: Call[] = []
  htui(on, answers, calls)
  await begin($, on, clock, calls)
  expect(await run($, 'waiting')).toBe(
    'MOD-4  gate  prd 0.1  18h  approve the PRD\nMOD-7  unblock  -  2d  blocked, no active run: u reopens it',
  )
  expect(calls).toEqual([{ argv: ['htui', 'waiting', '--json'], timeoutMs: 10000 }])
  answers.waiting = ok(WAITING_NONE)
  expect(await run($, 'waiting')).toBe('Nothing waits on you.')
})

test('waiting names judge_failed with a space', async ($, on) => {
  const clock = mock.clock(on)
  const doc = JSON.parse(WAITING)
  doc.rows = [{ ...doc.rows[0], reason: 'judge_failed', age_seconds: 42 }]
  htui(on, { waiting: ok(JSON.stringify(doc)) }, [])
  await begin($, on, clock)
  expect(await run($, 'waiting')).toBe('MOD-4  judge failed  prd 0.1  42s  approve the PRD')
})

test('search sends the query as one argument with a 60 s bound', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { search: ok(SEARCH) }, calls)
  await begin($, on, clock, calls)
  expect(await run($, '  search   closed items ')).toBe(
    'ANA-11  item  a decision is a closed item\nMOD-34  document prd (done)  the concepts index',
  )
  expect(calls[0]).toEqual({ argv: ['htui', '--search-items=closed items', '--json'], timeoutMs: 60000 })
})

test('a query that starts with a dash stays one argument', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { search: ok(SEARCH) }, calls)
  await begin($, on, clock, calls)
  await run($, 'search --json')
  expect(calls[0].argv).toEqual(['htui', '--search-items=--json', '--json'])
})

test('search with no hits says so', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { search: ok(JSON.stringify({ ...JSON.parse(SEARCH), hits: [] })) }, [])
  await begin($, on, clock)
  expect(await run($, 'search closed items')).toBe('No hits.')
})

test('search on an empty index says so', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { search: ok(SEARCH_EMPTY) }, [])
  await begin($, on, clock)
  expect(await run($, 'search closed items')).toBe('No concepts index yet: run htui --index-items.')
})

test('search with no query prints usage', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, {}, calls)
  await begin($, on, clock, calls)
  expect(await run($, 'search')).toBe('usage: /htui search <query>')
  expect(await run($, 'search   ')).toBe('usage: /htui search <query>')
  expect(calls).toHaveLength(0)
})

test('an unknown verb prints usage', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, {}, calls)
  await begin($, on, clock, calls)
  expect(await run($, 'frobnicate')).toBe('usage: /htui [status|waiting|search <query>]')
  expect(calls).toHaveLength(0)
})

test('an offline store exits 3 and shows its message', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { status: ok(OFFLINE, 3) }, [])
  await begin($, on, clock)
  expect(await run($, 'status')).toBe('htui: cannot reach Postgres: store unreachable: connection refused')
})

test('a refusal exits 2 and shows its message', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { waiting: ok(NO_DSN, 2) }, [])
  await begin($, on, clock)
  expect(await run($, 'waiting')).toBe('htui: no Postgres DSN is stored; run `htui --set-dsn` first')
})

test('a missing binary is a line, not a throw', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { status: 'deny' }, [])
  await begin($, on, clock)
  expect(await run($, 'status')).toBe('htui: not found on PATH')
})

test('a hung binary is a line, not a throw', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { status: 'hang' }, [], clock)
  await begin($, on, clock)
  const pending = run($, 'status')
  await clock.advance(10000)
  expect(await pending).toBe('htui: timed out')
})

test('empty stdout is a failure whatever the exit code', async ($, on) => {
  const clock = mock.clock(on)
  const answers: Record<string, Answer> = { status: ok('', 0) }
  htui(on, answers, [])
  await begin($, on, clock)
  expect(await run($, 'status')).toBe('htui printed nothing (exit 0)')
  answers.status = ok('  \n', 1)
  expect(await run($, 'status')).toBe('htui printed nothing (exit 1)')
})

test('an unreadable or newer answer is a line', async ($, on) => {
  const clock = mock.clock(on)
  const answers: Record<string, Answer> = { status: ok('htui 1.0\n', 0) }
  htui(on, answers, [])
  await begin($, on, clock)
  expect(await run($, 'status')).toBe('htui: unreadable answer (exit 0)')
  answers.status = ok('[1,2]', 0)
  expect(await run($, 'status')).toBe('htui: unreadable answer (exit 0)')
  answers.status = ok(JSON.stringify({ ...JSON.parse(STATUS), version: 2 }), 0)
  expect(await run($, 'status')).toBe(
    'htui: answer version 2 is newer than this plugin: run htui claude emit --force, then /reload-plugins',
  )
  answers.status = ok('{"version":1,"kind":"status","store":"error"}', 1)
  expect(await run($, 'status')).toBe('htui: exit 1')
})

test('waiting strips control characters from what htui stored', async ($, on) => {
  const clock = mock.clock(on)
  const doc = JSON.parse(WAITING)
  doc.rows = [
    { ...doc.rows[0], item_key: 'MOD-\u00074', step: 'prd\t0.1', text: 'line one\nline two \u001b[31mred\u001b[0m\r' },
    doc.rows[1],
  ]
  htui(on, { waiting: ok(JSON.stringify(doc)) }, [])
  await begin($, on, clock)
  expect(await run($, 'waiting')).toBe(
    'MOD-4  gate  prd0.1  18h  line oneline two [31mred[0m\nMOD-7  unblock  -  2d  blocked, no active run: u reopens it',
  )
})

test('status and search strip control characters', async ($, on) => {
  const clock = mock.clock(on)
  const status = JSON.parse(STATUS)
  status.workspace = { ...status.workspace, name: 'Plat\nform' }
  status.box = { ...status.box, hostname: 'de\u001bsk' }
  const search = JSON.parse(SEARCH)
  search.hits = [{ ...search.hits[1], key: 'MOD-\n34', document_kind: 'p\u0007rd', snippet: 'the\r\nconcepts index' }]
  htui(on, { status: ok(JSON.stringify(status)), search: ok(JSON.stringify(search)) }, [])
  await begin($, on, clock)
  expect(await run($, 'status')).toBe('Platform · desk · online · 1 working · 2 waiting')
  expect(await run($, 'search closed items')).toBe('MOD-34  document prd (done)  theconcepts index')
})

test('an error message is stripped of control characters', async ($, on) => {
  const clock = mock.clock(on)
  const doc = JSON.parse(NO_DSN)
  doc.error.message = 'no Postgres\nDSN \u001b[2Jis stored'
  htui(on, { waiting: ok(JSON.stringify(doc), 2) }, [])
  await begin($, on, clock)
  expect(await run($, 'waiting')).toBe('htui: no PostgresDSN [2Jis stored')
})

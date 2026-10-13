import { test, expect, mock } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'
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

/** Runs `/htui <args>` and answers its text. */
const run = async ($: Engine, args: string) =>
  (await $.command.run({ command: 'htui', args } as never)).text

test('status prints the top bar\'s line', async ($, on) => {
  mock.clock(on)
  const calls: Call[] = []
  htui(on, { status: ok(STATUS) }, calls)
  expect(await run($, 'status')).toBe('Platform · desk · online · 1 working · 2 waiting')
  expect(calls).toEqual([{ argv: ['htui', 'status', '--json'], timeoutMs: 10000 }])
})

test('status says when permission requests could not be listed', async ($, on) => {
  mock.clock(on)
  const doc = { ...JSON.parse(STATUS), workspace: null, box: null, permissions_known: false }
  htui(on, { status: ok(JSON.stringify(doc)) }, [])
  expect(await run($, 'status')).toBe(
    'no workspace · no box · online · 1 working · 2 waiting\npermission requests could not be listed; the count may be low',
  )
})

test('status shows the skew line', async ($, on) => {
  mock.clock(on)
  const calls: Call[] = []
  htui(on, { status: ok(STATUS), waiting: ok(WAITING), '--version': ok('htui 9.9.9\n') }, calls)
  await start($, on)
  expect(calls).toContainEqual({ argv: ['htui', '--version'], timeoutMs: 10000 })
  expect(await run($, 'status')).toBe(`Platform · desk · online · 1 working · 2 waiting\n${SKEW}`)
})

test('a matching binary shows no skew line', async ($, on) => {
  mock.clock(on)
  htui(on, { status: ok(STATUS), waiting: ok(WAITING), '--version': ok('htui 0.0.0-source\n') }, [])
  await start($, on)
  expect(await run($, 'status')).toBe('Platform · desk · online · 1 working · 2 waiting')
})

test('waiting prints one line per row, and a sentence when none', async ($, on) => {
  mock.clock(on)
  const answers: Record<string, Answer> = { waiting: ok(WAITING) }
  const calls: Call[] = []
  htui(on, answers, calls)
  expect(await run($, 'waiting')).toBe(
    'MOD-4  gate  prd 0.1  18h  approve the PRD\nMOD-7  unblock  -  2d  blocked, no active run: u reopens it',
  )
  expect(calls).toEqual([{ argv: ['htui', 'waiting', '--json'], timeoutMs: 10000 }])
  answers.waiting = ok(WAITING_NONE)
  expect(await run($, 'waiting')).toBe('Nothing waits on you.')
})

test('waiting names judge_failed with a space', async ($, on) => {
  mock.clock(on)
  const doc = JSON.parse(WAITING)
  doc.rows = [{ ...doc.rows[0], reason: 'judge_failed', age_seconds: 42 }]
  htui(on, { waiting: ok(JSON.stringify(doc)) }, [])
  expect(await run($, 'waiting')).toBe('MOD-4  judge failed  prd 0.1  42s  approve the PRD')
})

test('search sends the query as one argument with a 60 s bound', async ($, on) => {
  mock.clock(on)
  const calls: Call[] = []
  htui(on, { search: ok(SEARCH) }, calls)
  expect(await run($, '  search   closed items ')).toBe(
    'ANA-11  item  a decision is a closed item\nMOD-34  document prd (done)  the concepts index',
  )
  expect(calls[0]).toEqual({ argv: ['htui', '--search-items=closed items', '--json'], timeoutMs: 60000 })
})

test('a query that starts with a dash stays one argument', async ($, on) => {
  mock.clock(on)
  const calls: Call[] = []
  htui(on, { search: ok(SEARCH) }, calls)
  await run($, 'search --json')
  expect(calls[0].argv).toEqual(['htui', '--search-items=--json', '--json'])
})

test('search with no hits says so', async ($, on) => {
  mock.clock(on)
  htui(on, { search: ok(JSON.stringify({ ...JSON.parse(SEARCH), hits: [] })) }, [])
  expect(await run($, 'search closed items')).toBe('No hits.')
})

test('search on an empty index says so', async ($, on) => {
  mock.clock(on)
  htui(on, { search: ok(SEARCH_EMPTY) }, [])
  expect(await run($, 'search closed items')).toBe('No concepts index yet: run htui --index-items.')
})

test('search with no query prints usage', async ($, on) => {
  mock.clock(on)
  const calls: Call[] = []
  htui(on, {}, calls)
  expect(await run($, 'search')).toBe('usage: /htui search <query>')
  expect(await run($, 'search   ')).toBe('usage: /htui search <query>')
  expect(calls).toHaveLength(0)
})

test('an unknown verb prints usage', async ($, on) => {
  mock.clock(on)
  const calls: Call[] = []
  htui(on, {}, calls)
  expect(await run($, 'frobnicate')).toBe('usage: /htui [status|waiting|search <query>]')
  expect(calls).toHaveLength(0)
})

test('an offline store exits 3 and shows its message', async ($, on) => {
  mock.clock(on)
  htui(on, { status: ok(OFFLINE, 3) }, [])
  expect(await run($, 'status')).toBe('htui: cannot reach Postgres: store unreachable: connection refused')
})

test('a refusal exits 2 and shows its message', async ($, on) => {
  mock.clock(on)
  htui(on, { waiting: ok(NO_DSN, 2) }, [])
  expect(await run($, 'waiting')).toBe('htui: no Postgres DSN is stored; run `htui --set-dsn` first')
})

test('a missing binary is a line, not a throw', async ($, on) => {
  mock.clock(on)
  htui(on, { status: 'deny' }, [])
  expect(await run($, 'status')).toBe('htui: not found on PATH')
})

test('a hung binary is a line, not a throw', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { status: 'hang' }, [], clock)
  const pending = run($, 'status')
  await clock.advance(10000)
  expect(await pending).toBe('htui: timed out')
})

test('empty stdout is a failure whatever the exit code', async ($, on) => {
  mock.clock(on)
  const answers: Record<string, Answer> = { status: ok('', 0) }
  htui(on, answers, [])
  expect(await run($, 'status')).toBe('htui printed nothing (exit 0)')
  answers.status = ok('  \n', 1)
  expect(await run($, 'status')).toBe('htui printed nothing (exit 1)')
})

test('an unreadable or newer answer is a line', async ($, on) => {
  mock.clock(on)
  const answers: Record<string, Answer> = { status: ok('htui 1.0\n', 0) }
  htui(on, answers, [])
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

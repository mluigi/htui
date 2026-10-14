import { test, expect } from 'claude-code/testing'
import {
  age,
  composeAmbient,
  composeLine,
  gateStatusText,
  gateSummary,
  gateToast,
  gather,
  isFreshRun,
  itemOf,
  parseGate,
  parseHealth,
  parsePorcelainV2,
  phaseOf,
  type Ran,
  type Snapshot,
} from '../hooks/status'

const HEAD = 'a'.repeat(40)
const T0 = Date.parse('2026-10-14T12:00:00Z')
const iso = (ms: number) => new Date(ms).toISOString().replace(/\.\d{3}Z$/, 'Z')

/** A `last.json` as gate.sh writes it, `started`/`finished` given as seconds before T0. */
const gateDoc = (o: {
  state: 'running' | 'passed' | 'failed'
  startedAgo: number
  finishedAgo?: number
  head?: string
  steps?: [string, string, number][]
}) =>
  JSON.stringify({
    head: o.head ?? HEAD,
    started: iso(T0 - o.startedAgo * 1000),
    finished: o.finishedAgo === undefined ? null : iso(T0 - o.finishedAgo * 1000),
    state: o.state,
    steps: (o.steps ?? []).map(([name, status, seconds]) => ({ name, status, seconds })),
  })

const HANDOFF = [
  '# HANDOFF',
  '',
  '### Tooling findings',
  '',
  '- [ ] **TOOL-8 - `htui-dev` mod: guards** (from ANA-29). Body text.',
  '  more body. **Phase M1 landed (`1d3963c3`, 2026-10-14):** guards.',
  '  **Phase M2 landed (2026-10-15):** status line.',
  '- [ ] **TOOL-80 - another item.** **Phase Z landed (x):** not TOOL-8.',
  '- [ ] **TOOL-9 - no phase yet.** plain body',
  '- [ ] **TOOL-10 - next item.** **Phase Q landed (y):** belongs to TOOL-10.',
  '- [x] **TOOL-7 - done item.** **Phase D landed (z):** closed.',
  '- [ ] **TOOL-11 - before a heading.** body',
  '',
  '## Summary',
  '',
  'stray **Phase S landed (s):** under a heading.',
].join('\n')

test('HR_ITEM wins over a key in the branch', () => {
  expect(itemOf('TOOL-8', 'hr/MOD-4')).toEqual({ label: 'TOOL-8', key: 'TOOL-8' })
})

test('the first key in the branch is the item, upper-cased', () => {
  expect(itemOf(undefined, 'hr/tool-8')).toEqual({ label: 'TOOL-8', key: 'TOOL-8' })
  expect(itemOf('', 'x/mod-12-mod-13')).toEqual({ label: 'MOD-12', key: 'MOD-12' })
})

test('a branch with no key is the item alone; a detached HEAD reads detached', () => {
  expect(itemOf(undefined, 'main')).toEqual({ label: 'main', key: null })
  expect(itemOf(undefined, null)).toEqual({ label: 'detached', key: null })
})

test('an open item with phase notes reads the last one as landed', () => {
  expect(phaseOf(HANDOFF, 'TOOL-8')).toBe('M2 landed')
})

test('an open item with no phase note reads open', () => {
  expect(phaseOf(HANDOFF, 'TOOL-9')).toBe('open')
  expect(phaseOf(HANDOFF, 'TOOL-11')).toBe('open')
})

test("a phase note under the next item is not this item's", () => {
  expect(phaseOf(HANDOFF, 'TOOL-9')).toBe('open')
  expect(phaseOf(HANDOFF, 'TOOL-10')).toBe('Q landed')
})

test("TOOL-8 does not match TOOL-80's line", () => {
  expect(phaseOf(HANDOFF, 'TOOL-80')).toBe('Z landed')
  expect(phaseOf(HANDOFF.replace(/- \[ \] \*\*TOOL-8 -/, '- [x] **TOOL-8 -'), 'TOOL-8')).toBe('not open')
})

test('a closed or missing item reads not open', () => {
  expect(phaseOf(HANDOFF, 'TOOL-7')).toBe('not open')
  expect(phaseOf(HANDOFF, 'MOD-999')).toBe('not open')
})

test('a heading ends the body', () => {
  expect(phaseOf(HANDOFF, 'TOOL-11')).toBe('open')
})

test('CRLF line endings parse as LF', () => {
  expect(phaseOf(HANDOFF.replace(/\n/g, '\r\n'), 'TOOL-8')).toBe('M2 landed')
  expect(phaseOf(HANDOFF.replace(/\n/g, '\r\n'), 'TOOL-9')).toBe('open')
})

test('a key with regex characters is matched literally', () => {
  expect(phaseOf(HANDOFF, 'TOOL.8')).toBe('not open')
})

test('porcelain with branch.ab reads ahead, oid and head', () => {
  const p = parsePorcelainV2(`# branch.oid ${HEAD}\n# branch.head hr/TOOL-8\n# branch.upstream origin/x\n# branch.ab +3 -2\n`)
  expect(p).toEqual({ oid: HEAD, head: 'hr/TOOL-8', ahead: 3, dirty: 0 })
})

test('porcelain with no upstream reads ahead null; detached and initial read null', () => {
  expect(parsePorcelainV2('# branch.oid (initial)\n# branch.head (detached)\n')).toEqual({
    oid: null,
    head: null,
    ahead: null,
    dirty: 0,
  })
})

test('dirty counts changed, renamed, unmerged and untracked entries', () => {
  const out = [
    `# branch.oid ${HEAD}`,
    '# branch.head main',
    '1 .M N... 100644 100644 100644 a b f1',
    '2 R. N... 100644 100644 100644 a b R100 new\told',
    'u AA N... 000000 100644 100644 100644 a b c f3',
    '? untracked',
    '! ignored',
    '',
  ].join('\n')
  expect(parsePorcelainV2(out).dirty).toBe(4)
})

test('age switches unit at a minute, an hour and a day', () => {
  expect([age(0), age(59), age(60), age(3599), age(3600), age(86399), age(86400)]).toEqual([
    '0s',
    '59s',
    '1m',
    '59m',
    '1h',
    '23h',
    '1d',
  ])
})

test('a running gate reads its age from started', () => {
  expect(gateSummary(parseGate(gateDoc({ state: 'running', startedAgo: 240 })), HEAD, T0)).toBe('gate running 4m')
})

test('a passed gate reads its age from finished', () => {
  const doc = parseGate(gateDoc({ state: 'passed', startedAgo: 9000, finishedAgo: 7200 }))
  expect(gateSummary(doc, HEAD, T0)).toBe('gate passed 2h')
})

test('a failed gate lists its failed steps in order', () => {
  const doc = parseGate(
    gateDoc({
      state: 'failed',
      startedAgo: 5000,
      finishedAgo: 2400,
      steps: [
        ['fmt', 'passed', 1],
        ['test', 'failed', 900],
        ['abort-grep', 'failed', 1],
        ['plugin', 'skipped', 0],
      ],
    }),
  )
  expect(gateSummary(doc, HEAD, T0)).toBe('gate failed: test, abort-grep 40m')
})

test('a result for another HEAD says so', () => {
  const doc = parseGate(gateDoc({ state: 'passed', startedAgo: 200, finishedAgo: 100, head: 'b'.repeat(40) }))
  expect(gateSummary(doc, HEAD, T0)).toBe('gate passed 1m (older HEAD)')
  expect(gateSummary(doc, null, T0)).toBe('gate passed 1m')
})

test('an absent last.json shows nothing', () => {
  expect(parseGate(null)).toBe(null)
  expect(gateSummary(null, HEAD, T0)).toBe(null)
})

test('a malformed last.json reads unreadable', () => {
  for (const text of ['{', '[]', '{"state":"weird"}', JSON.stringify({ state: 'passed', steps: 'x' })]) {
    expect(parseGate(text), text).toBe('unreadable')
  }
  expect(gateSummary('unreadable', HEAD, T0)).toBe('gate: last.json unreadable')
})

test('an unparsable date drops the age', () => {
  const doc = parseGate(JSON.stringify({ head: HEAD, started: 'x', finished: null, state: 'running', steps: [] }))
  expect(gateSummary(doc, HEAD, T0)).toBe('gate running')
})

test('a toast names the outcome; a running gate has none', () => {
  const failed = parseGate(gateDoc({ state: 'failed', startedAgo: 9, finishedAgo: 1, steps: [['test', 'failed', 5]] }))
  const passed = parseGate(gateDoc({ state: 'passed', startedAgo: 9, finishedAgo: 1 }))
  const running = parseGate(gateDoc({ state: 'running', startedAgo: 9 }))
  if (failed === null || failed === 'unreadable' || passed === null || passed === 'unreadable') throw new Error('unparsed')
  if (running === null || running === 'unreadable') throw new Error('unparsed')
  expect(gateToast(failed)).toBe('gate failed: test')
  expect(gateToast(passed)).toBe('gate passed')
  expect(gateToast(running)).toBe(null)
})

test('a run is fresh while running and started under 4 h ago', () => {
  expect(isFreshRun(parseGate(gateDoc({ state: 'running', startedAgo: 3600 })), T0)).toBe(true)
  expect(isFreshRun(parseGate(gateDoc({ state: 'running', startedAgo: 4 * 3600 + 1 })), T0)).toBe(false)
  expect(isFreshRun(parseGate(gateDoc({ state: 'passed', startedAgo: 9, finishedAgo: 1 })), T0)).toBe(false)
  expect(isFreshRun('unreadable', T0)).toBe(false)
  expect(isFreshRun(null, T0)).toBe(false)
})

test('the gate status text lists each step', () => {
  const doc = parseGate(
    gateDoc({ state: 'failed', startedAgo: 999, finishedAgo: 60, steps: [['fmt', 'passed', 3], ['test', 'failed', 125]] }),
  )
  expect(gateStatusText(doc, HEAD, T0)).toBe('gate failed: test 1m\nfmt passed 3s\ntest failed 2m')
  expect(gateStatusText(null, HEAD, T0)).toBe('no gate result yet')
})

const snap = (o: Partial<Snapshot> = {}): Snapshot => ({
  item: { label: 'TOOL-8', key: 'TOOL-8' },
  phase: 'M1 landed',
  merging: false,
  ahead: 0,
  dirty: 0,
  health: { usedPct: 80, pg: 'up' },
  gate: null,
  head: HEAD,
  ...o,
})

test('the ambient line orders its parts and drops the quiet ones', () => {
  expect(composeAmbient(snap(), T0)).toBe('TOOL-8 M1 landed')
  const gate = parseGate(gateDoc({ state: 'passed', startedAgo: 9000, finishedAgo: 7200 }))
  expect(
    composeAmbient(snap({ merging: true, ahead: 2, dirty: 1, health: { usedPct: 93, pg: 'down' }, gate }), T0),
  ).toBe('TOOL-8 M1 landed · merging · 2 ahead · 1 dirty · disk 93% · pg down · gate passed 2h')
  expect(composeAmbient(snap({ health: { usedPct: 90, pg: 'recovering' } }), T0)).toBe('TOOL-8 M1 landed · pg recovering')
  expect(composeAmbient(snap({ phase: null, health: { usedPct: null, pg: 'unknown' } }), T0)).toBe('TOOL-8')
  expect(composeAmbient(snap({ ahead: null, dirty: null, health: { usedPct: null, pg: null } }), T0)).toBe('TOOL-8 M1 landed')
})

test('a warning leads the line; either alone is the line; neither clears it', () => {
  expect(composeLine('w', 'a')).toBe('⚠ w · a')
  expect(composeLine('w', null)).toBe('w')
  expect(composeLine(null, 'a')).toBe('a')
  expect(composeLine(null, null)).toBe(undefined)
})

test('malformed dev-health output reads as nothing', () => {
  expect(parseHealth('{"disk":{"used_pct":93},"postgres":{"state":"down"}}')).toEqual({ usedPct: 93, pg: 'down' })
  expect(parseHealth('{"disk":{"used_pct":null},"postgres":{"state":"weird"}}')).toEqual({ usedPct: null, pg: null })
  expect(parseHealth('not json')).toEqual({ usedPct: null, pg: null })
})

/** A fake runner: answers by the argv's joined text, records each call. */
const fakeRun = (answers: Record<string, Ran>, calls: string[][]) => async (argv: readonly string[]) => {
  calls.push([...argv])
  const key = Object.keys(answers).find(k => argv.join(' ').includes(k))
  return key === undefined ? ({ failed: 'not found' } as Ran) : answers[key]
}
const okRan = (stdout: string, exitCode = 0): Ran => ({ exitCode, stdout, stderr: '' })
const HOST = { root: '/repo', windows: false }
const HEALTH = ['bash', '/repo/scripts/dev-health.sh']

test('gather counts commits ahead of main only when there is no upstream off main', async () => {
  const calls: string[][] = []
  const read = async (p: string) => (p === '/repo/HANDOFF.md' ? HANDOFF : null)
  const answers = {
    'status --porcelain': okRan(`# branch.oid ${HEAD}\n# branch.head hr/TOOL-8\n? x\n`),
    MERGE_HEAD: okRan('', 1),
    'rev-list': okRan('5\n'),
    'dev-health': okRan('{"disk":{"used_pct":40},"postgres":{"state":"up"}}'),
  }
  const s = await gather(HOST, undefined, fakeRun(answers, calls), read, HEALTH)
  expect(s).toEqual({
    item: { label: 'TOOL-8', key: 'TOOL-8' },
    phase: 'M2 landed',
    merging: false,
    ahead: 5,
    dirty: 1,
    health: { usedPct: 40, pg: 'up' },
    gate: null,
    head: HEAD,
  })
  expect(calls.find(c => c.includes('rev-list'))).toEqual(['git', 'rev-list', '--count', 'main..HEAD'])
  expect(calls[0]).toEqual(['git', '--no-optional-locks', 'status', '--porcelain=v2', '--branch'])

  const onMain: string[][] = []
  const mainAnswers = { ...answers, 'status --porcelain': okRan(`# branch.oid ${HEAD}\n# branch.head main\n`) }
  const m = await gather(HOST, undefined, fakeRun(mainAnswers, onMain), read, HEALTH)
  expect(m.ahead).toBe(null)
  expect(onMain.some(c => c.includes('rev-list'))).toBe(false)

  const tracked: string[][] = []
  const upAnswers = { ...answers, 'status --porcelain': okRan(`# branch.oid ${HEAD}\n# branch.head f\n# branch.ab +2 -0\n`) }
  expect((await gather(HOST, undefined, fakeRun(upAnswers, tracked), read, HEALTH)).ahead).toBe(2)
  expect(tracked.some(c => c.includes('rev-list'))).toBe(false)
})

test('gather keeps going when every piece fails', async () => {
  const s = await gather(HOST, 'TOOL-8', fakeRun({}, []), async () => null, HEALTH)
  expect(s).toEqual({
    item: { label: 'TOOL-8', key: 'TOOL-8' },
    phase: null,
    merging: false,
    ahead: null,
    dirty: null,
    health: { usedPct: null, pg: null },
    gate: null,
    head: null,
  })
})

test('gather reads merging, the gate file, and the phase from the HANDOFF', async () => {
  const files: Record<string, string> = {
    '/repo/HANDOFF.md': HANDOFF,
    '/repo/target/gate/last.json': gateDoc({ state: 'running', startedAgo: 60 }),
  }
  const answers = {
    'status --porcelain': okRan(`# branch.oid ${HEAD}\n# branch.head main\n`),
    MERGE_HEAD: okRan(`${HEAD}\n`),
  }
  const s = await gather(HOST, 'TOOL-9', fakeRun(answers, []), async p => files[p] ?? null, HEALTH)
  expect(s.merging).toBe(true)
  expect(s.phase).toBe('open')
  expect(composeAmbient(s, T0)).toBe('TOOL-9 open · merging · gate running 1m')
})

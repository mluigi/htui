import { test, expect, mock } from 'claude-code/testing'
import { BAND, engineBottom, htui, ok, start, STATUS, waitingCalls, WAITING, WAITING_NONE, type Answer, type Call } from './fixtures'

test('auto polls at start and every poll_seconds', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { waiting: ok(WAITING) }, calls)
  await start($, on)
  await clock.settle()
  expect(waitingCalls(calls)).toBe(1)
  expect(calls.find(c => c.argv[1] === 'waiting')).toEqual({ argv: ['htui', 'waiting', '--json'], timeoutMs: 10000 })
  await clock.advance(29999)
  expect(waitingCalls(calls)).toBe(1)
  await clock.advance(1)
  expect(waitingCalls(calls)).toBe(2)
})

// F7a: poll_seconds is clamped to [5, 3600]; 0 reads as the default 30. A non-number never
// reaches the mod: the engine refuses options that do not fit plugin.json's userConfig.
const PERIODS: [string, number, number][] = [
  ['below 5 is read as 5', 1, 5],
  ['4 is read as 5', 4, 5],
  ['a negative is read as 5', -10, 5],
  ['0 is read as 30', 0, 30],
  ['above 3600 is read as 3600', 7200, 3600],
  ['3600 is kept', 3600, 3600],
  ['7 is kept', 7, 7],
]

for (const [name, poll_seconds, seconds] of PERIODS) {
  test(`poll_seconds ${name}`, { options: { poll_seconds } }, async ($, on) => {
    const clock = mock.clock(on)
    const calls: Call[] = []
    htui(on, { waiting: ok(WAITING) }, calls)
    await start($, on)
    await clock.settle()
    expect(waitingCalls(calls)).toBe(1)
    await clock.advance(seconds * 1000 - 1)
    expect(waitingCalls(calls)).toBe(1)
    await clock.advance(1)
    expect(waitingCalls(calls)).toBe(2)
  })
}

test('command polls nothing until /htui and stops when the pane closes', { options: { panel: 'command' } }, async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { waiting: ok(WAITING) }, calls)
  let isOpen = false
  on('ui.open', async () => {
    isOpen = true
    return { value: { isPlaced: true } }
  })
  let panes = 0
  on('ui.panes', async () => {
    panes += 1
    return { value: isOpen ? [{ id: 'htui', title: 'htui', isShown: true, isFocused: false, isPlaced: true }] : [] }
  })
  await start($, on)
  await clock.settle()
  await clock.advance(30000)
  expect(waitingCalls(calls)).toBe(0)
  expect(panes).toBe(0)
  const r = await $.command.run({ command: 'htui', args: '' } as never)
  expect(r.text).toBeUndefined()
  await clock.settle()
  expect(waitingCalls(calls)).toBe(1)
  await clock.advance(30000)
  expect(waitingCalls(calls)).toBe(2)
  expect(panes).toBe(1)
  isOpen = false
  // The first tick after the close finds the pane gone and cancels the timer: no tick follows.
  await clock.advance(30000)
  expect(panes).toBe(2)
  await clock.advance(30000)
  await clock.advance(30000)
  expect(panes).toBe(2)
  expect(waitingCalls(calls)).toBe(2)
  // /htui again starts a new timer.
  await $.command.run({ command: 'htui', args: '' } as never)
  await clock.settle()
  expect(waitingCalls(calls)).toBe(3)
  await clock.advance(30000)
  expect(panes).toBe(3)
  expect(waitingCalls(calls)).toBe(4)
})

test('off never polls and never opens a pane', { options: { panel: 'off' } }, async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { waiting: ok(WAITING) }, calls)
  let opens = 0
  on('ui.open', async () => {
    opens += 1
    return { value: { isPlaced: true } }
  })
  await start($, on)
  await clock.settle()
  await clock.advance(60000)
  expect(waitingCalls(calls)).toBe(0)
  const r = await $.command.run({ command: 'htui', args: '' } as never)
  expect(r.text).toBe('MOD-4  gate  prd 0.1  18h  approve the PRD\nMOD-7  unblock  -  2d  blocked, no active run: u reopens it')
  expect(waitingCalls(calls)).toBe(1)
  expect(opens).toBe(0)
})

test('an htui-driven session stands down', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { waiting: ok(WAITING) }, calls)
  engineBottom(on, 'AbovePrompt')
  let registers = 0
  on('command.register', async () => {
    registers += 1
    return { value: undefined }
  })
  mock.env(on, { HTUI_SESSION: '1' })
  on('session.start', async ($, e) => e)
  await $.session.start({ source: 'startup', cwd: '/tmp' } as never)
  await clock.advance(60000)
  expect(registers).toBe(0)
  expect(calls).toHaveLength(0)
  for (const surface of ['terminal', 'desktop'] as const) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
    expect(await ui.find({ key: 'engine' })).toBeDefined()
    expect(await ui.find({ key: 'htui-band' })).toBeUndefined()
    await ui.unmount()
  }
})

test('a slow poll is not overlapped', { options: { poll_seconds: 5 } }, async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { waiting: 'hang' }, calls, clock)
  await start($, on)
  await clock.settle()
  expect(waitingCalls(calls)).toBe(1)
  await clock.advance(5000)
  expect(waitingCalls(calls)).toBe(1)
  await clock.advance(4999)
  expect(waitingCalls(calls)).toBe(1)
  // The hang answers at 10 s, before that period's tick, which polls again; that poll hangs in turn.
  await clock.advance(1)
  expect(waitingCalls(calls)).toBe(2)
  await clock.advance(5000)
  expect(waitingCalls(calls)).toBe(2)
})

const LATER: [string, Answer][] = [
  ['a matching binary', ok('htui 0.0.0-source\n')],
  ['a failed --version', ok('', 1)],
  ['an unparsed --version', ok('no version here\n')],
  ['a missing binary', 'deny'],
]

for (const [name, later] of LATER) {
  test(`a later session.start with ${name} clears the skew line`, async ($, on) => {
    const clock = mock.clock(on)
    engineBottom(on, 'AbovePrompt')
    const answers: Record<string, Answer> = { status: ok(STATUS), waiting: ok(WAITING_NONE), '--version': ok('htui 9.9.9\n') }
    htui(on, answers, [])
    await start($, on)
    await clock.settle()
    const status = async () => (await $.command.run({ command: 'htui', args: 'status' } as never)).text
    const band = async (surface: 'terminal' | 'desktop') => {
      const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
      const found = (await ui.find({ key: 'htui-band' })) !== undefined
      // A-3: a band that stands down leaves the engine's own drawing in place.
      if (!found) expect(await ui.find({ key: 'engine' })).toBeDefined()
      await ui.unmount()
      return found
    }
    expect(await status()).toContain('htui 9.9.9 ≠ plugin 0.0.0-source')
    for (const surface of ['terminal', 'desktop'] as const) expect(await band(surface)).toBe(true)
    // After `htui claude emit --force`, `/reload-plugins` raises session.start again; `$.state` survives it.
    answers['--version'] = later
    await $.session.start({ source: 'startup', cwd: '/tmp' } as never)
    await clock.settle()
    expect(await status()).toBe('Platform · desk · online · 1 working · 2 waiting')
    for (const surface of ['terminal', 'desktop'] as const) expect(await band(surface)).toBe(false)
  })
}

import { test, expect, mock } from 'claude-code/testing'
import { BAND, engineBottom, htui, ok, start, waitingCalls, WAITING, type Call } from './fixtures'

test('auto polls at start and every poll_seconds', async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { waiting: ok(WAITING) }, calls)
  await start($, on)
  expect(waitingCalls(calls)).toBe(1)
  expect(calls.find(c => c.argv[1] === 'waiting')).toEqual({ argv: ['htui', 'waiting', '--json'], timeoutMs: 10000 })
  await clock.advance(29999)
  expect(waitingCalls(calls)).toBe(1)
  await clock.advance(1)
  expect(waitingCalls(calls)).toBe(2)
})

test('poll_seconds below 5 is read as 5', { options: { poll_seconds: 1 } }, async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { waiting: ok(WAITING) }, calls)
  await start($, on)
  expect(waitingCalls(calls)).toBe(1)
  await clock.advance(5000)
  expect(waitingCalls(calls)).toBe(2)
  await clock.advance(1000)
  expect(waitingCalls(calls)).toBe(2)
})

test('command polls nothing until /htui and stops when the pane closes', { options: { panel: 'command' } }, async ($, on) => {
  const clock = mock.clock(on)
  const calls: Call[] = []
  htui(on, { waiting: ok(WAITING) }, calls)
  let isOpen = false
  on('ui.open', async () => {
    isOpen = true
    return { value: { isPlaced: true } }
  })
  on('ui.panes', async () => ({
    value: isOpen ? [{ id: 'htui', title: 'htui', isShown: true, isFocused: false, isPlaced: true }] : [],
  }))
  await start($, on)
  await clock.advance(30000)
  expect(waitingCalls(calls)).toBe(0)
  const r = await $.command.run({ command: 'htui', args: '' } as never)
  expect(r.text).toBeUndefined()
  await clock.settle()
  expect(waitingCalls(calls)).toBe(1)
  await clock.advance(30000)
  expect(waitingCalls(calls)).toBe(2)
  isOpen = false
  await clock.advance(30000)
  await clock.advance(30000)
  expect(waitingCalls(calls)).toBe(2)
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

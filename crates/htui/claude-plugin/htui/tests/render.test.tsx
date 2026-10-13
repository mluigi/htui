import { test, expect, mock } from 'claude-code/testing'
import { BAND, engineBottom, htui, OFFLINE, ok, PANE, start, WAITING, WAITING_NONE, type Answer } from './fixtures'

const SURFACES = ['terminal', 'desktop'] as const

test('the band shows the count on terminal and desktop', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { waiting: ok(WAITING) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
    expect(await ui.find({ key: 'htui-band' })).toBeDefined()
    expect(await ui.find({ type: 'Text', text: /^htui: 2 waiting on you$/ })).toBeDefined()
    expect(await ui.findAll({ type: 'Text' })).toHaveLength(1)
    await ui.unmount()
  }
})

test('no rows and no skew leaves the band to the engine', async ($, on) => {
  const clock = mock.clock(on)
  engineBottom(on, 'AbovePrompt')
  htui(on, { waiting: ok(WAITING_NONE) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
    expect(await ui.find({ key: 'engine' })).toBeDefined()
    expect(await ui.find({ key: 'htui-band' })).toBeUndefined()
    await ui.unmount()
  }
})

test('the band is the engine\'s before the first poll answers', async ($, on) => {
  const clock = mock.clock(on)
  engineBottom(on, 'AbovePrompt')
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
    expect(await ui.find({ key: 'engine' })).toBeDefined()
    expect(await ui.find({ key: 'htui-band' })).toBeUndefined()
    await ui.unmount()
  }
})

test('a survey keeps the band away', async ($, on) => {
  const clock = mock.clock(on)
  engineBottom(on, 'AbovePrompt')
  htui(on, { waiting: ok(WAITING) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const props = { ...(BAND as object), hasSurvey: true } as never
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props })
    expect(await ui.find({ key: 'engine' })).toBeDefined()
    expect(await ui.find({ key: 'htui-band' })).toBeUndefined()
    await ui.unmount()
  }
})

test('a failed poll shows its line in the band', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { waiting: ok(OFFLINE, 3) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
    expect(await ui.find({ key: 'htui-band' })).toBeDefined()
    const line = await ui.find({ type: 'Text', text: /cannot reach Postgres/ })
    expect(line?.text).toBe('htui: cannot reach Postgres: store unreachable: connection refused')
    expect(await ui.find({ type: 'Text', text: /waiting on you/ })).toBeUndefined()
    await ui.unmount()
  }
})

test('a failed poll drops the rows an earlier poll showed', async ($, on) => {
  const clock = mock.clock(on)
  const answers: Record<string, Answer> = { waiting: ok(WAITING) }
  htui(on, answers, [])
  await start($, on)
  await clock.settle()
  answers.waiting = ok(OFFLINE, 3)
  await clock.advance(30000)
  for (const surface of SURFACES) {
    const band = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
    expect(await band.find({ type: 'Text', text: /waiting on you/ })).toBeUndefined()
    const line = await band.find({ type: 'Text', text: /cannot reach Postgres/ })
    expect(line?.text).toBe('htui: cannot reach Postgres: store unreachable: connection refused')
    await band.unmount()
    const pane = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props: PANE, requestId: 'htui' })
    expect(await pane.find({ key: 'row-0' })).toBeUndefined()
    // V1: the header states no count as fact after a failed poll; the error line beneath says why.
    expect((await pane.findAll({ type: 'Text' })).map(t => t.text)).toEqual([
      'htui',
      'htui: cannot reach Postgres: store unreachable: connection refused',
    ])
    await pane.unmount()
  }
})

test('a pane row is stripped of control characters', async ($, on) => {
  const clock = mock.clock(on)
  const doc = JSON.parse(WAITING)
  doc.workspace = { ...doc.workspace, name: 'Plat\nform' }
  doc.rows = [{ ...doc.rows[0], text: 'line one\nline two \u001b[31mred' }]
  htui(on, { waiting: ok(JSON.stringify(doc)) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const props = { ...(PANE as object), bodyColumns: 100 } as never
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props, requestId: 'htui' })
    expect((await ui.find({ key: 'row-0' }))?.text).toBe('gate         MOD-4     prd 0.1          18h  line oneline two [31mred')
    expect((await ui.find({ type: 'Text', text: /waiting$/ }))?.text).toBe('Platform · 1 waiting')
    await ui.unmount()
  }
})

test('the band shows the skew line when nothing waits', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { waiting: ok(WAITING_NONE), '--version': ok('htui 9.9.9\n') }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
    const line = await ui.find({ type: 'Text', text: /≠ plugin/ })
    expect(line?.text).toBe('htui 9.9.9 ≠ plugin 0.0.0-source: run htui claude emit --force, then /reload-plugins')
    await ui.unmount()
  }
})

for (const panel of ['command', 'off'] as const) {
  test(`${panel} draws no band`, { options: { panel } }, async ($, on) => {
    const clock = mock.clock(on)
    engineBottom(on, 'AbovePrompt')
    htui(on, { waiting: ok(WAITING), '--version': ok('htui 9.9.9\n') }, [])
    on('ui.open', async () => ({ value: { isPlaced: true } }))
    await start($, on)
    await clock.settle()
    await $.command.run({ command: 'htui', args: '' } as never)
    for (const surface of SURFACES) {
      const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
      expect(await ui.find({ key: 'engine' })).toBeDefined()
      expect(await ui.find({ key: 'htui-band' })).toBeUndefined()
      await ui.unmount()
    }
  })
}

test('the pane lists rows cut to its width', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { waiting: ok(WAITING) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const props = { ...(PANE as object), bodyColumns: 40 } as never
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props, requestId: 'htui' })
    expect(await ui.find({ key: 'htui-pane' })).toBeDefined()
    expect(await ui.find({ key: 'row-0' })).toBeDefined()
    expect(await ui.find({ key: 'row-1' })).toBeDefined()
    expect(await ui.find({ key: 'row-2' })).toBeUndefined()
    const texts = await ui.findAll({ type: 'Text' })
    expect(texts.map(t => t.text)).toEqual([
      'Platform · 2 waiting',
      'gate         MOD-4     prd 0.1         …',
      'unblock      MOD-7     -               …',
    ])
    for (const t of texts) expect(t.text.length).toBeLessThanOrEqual(40)
    await ui.unmount()
  }
})

test('a wide pane shows each row whole', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { waiting: ok(WAITING) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props: PANE, requestId: 'htui' })
    const row = await ui.find({ key: 'row-0' })
    expect(row?.text).toBe('gate         MOD-4     prd 0.1          18h  approve the PRD')
    await ui.unmount()
  }
})

test('an empty pane says nothing waits', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { waiting: ok(WAITING_NONE) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props: PANE, requestId: 'htui' })
    expect(await ui.find({ type: 'Text', text: 'Nothing waits on you.' })).toBeDefined()
    expect(await ui.find({ key: 'row-0' })).toBeUndefined()
    await ui.unmount()
  }
})

test('the pane says when no poll has answered yet', { options: { panel: 'command' } }, async ($, on) => {
  const clock = mock.clock(on)
  htui(on, {}, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props: PANE, requestId: 'htui' })
    // V1: before the first answer the header states no count as fact.
    expect((await ui.findAll({ type: 'Text' })).map(t => t.text)).toEqual(['htui', 'htui: no answer yet'])
    await ui.unmount()
  }
})

const SKEW = 'htui 9.9.9 ≠ plugin 0.0.0-source: run htui claude emit --force, then /reload-plugins'

test('the pane shows the skew line and the error line', async ($, on) => {
  const clock = mock.clock(on)
  const answers: Record<string, Answer> = { waiting: ok(OFFLINE, 3), '--version': ok('htui 9.9.9\n') }
  htui(on, answers, [])
  await start($, on)
  await clock.settle()
  const props = { ...(PANE as object), bodyColumns: 200 } as never
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props, requestId: 'htui' })
    expect((await ui.findAll({ type: 'Text' })).map(t => t.text)).toEqual([
      'htui',
      SKEW,
      'htui: cannot reach Postgres: store unreachable: connection refused',
    ])
    await ui.unmount()
  }
  answers.waiting = ok(WAITING)
  await clock.advance(30000)
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props, requestId: 'htui' })
    expect((await ui.findAll({ type: 'Text' })).map(t => t.text)).toEqual([
      'Platform · 2 waiting',
      SKEW,
      'gate         MOD-4     prd 0.1          18h  approve the PRD',
      'unblock      MOD-7     -                 2d  blocked, no active run: u reopens it',
    ])
    await ui.unmount()
  }
})

test('a poll whose rows is not a list shows a line, not a count', async ($, on) => {
  const clock = mock.clock(on)
  htui(on, { waiting: ok(JSON.stringify({ ...JSON.parse(WAITING), rows: 'none' })) }, [])
  await start($, on)
  await clock.settle()
  for (const surface of SURFACES) {
    const band = await $.ui.mount({ plugin: 'htui', surface, component: 'AbovePrompt', props: BAND })
    expect((await band.findAll({ type: 'Text' })).map(t => t.text)).toEqual(['htui: unreadable answer (exit 0)'])
    await band.unmount()
    const pane = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props: PANE, requestId: 'htui' })
    expect((await pane.findAll({ type: 'Text' })).map(t => t.text)).toEqual(['htui', 'htui: unreadable answer (exit 0)'])
    await pane.unmount()
  }
})

test('pane columns never run together', async ($, on) => {
  const clock = mock.clock(on)
  const doc = JSON.parse(WAITING)
  doc.rows = [
    { ...doc.rows[0], reason: 'permission_request', item_key: 'LONGPREFIX-12345', step: 'a very long step name', age_seconds: 5 },
    { ...doc.rows[0], reason: 'abcdefghijklm', item_key: 'ABCDEFGHIJ', step: 'sixteen-chars-xx', age_seconds: 950400 },
  ]
  htui(on, { waiting: ok(JSON.stringify(doc)) }, [])
  await start($, on)
  await clock.settle()
  const props = { ...(PANE as object), bodyColumns: 200 } as never
  for (const surface of SURFACES) {
    const ui = await $.ui.mount({ plugin: 'htui', surface, component: 'Pane', props, requestId: 'htui' })
    expect((await ui.find({ key: 'row-0' }))?.text).toBe(
      'permission request LONGPREFIX-12345 a very long step name   5s  approve the PRD',
    )
    expect((await ui.find({ key: 'row-1' }))?.text).toBe('abcdefghijklm ABCDEFGHIJ sixteen-chars-xx  11d  approve the PRD')
    await ui.unmount()
  }
})

// htui's mod: a band above the prompt when something waits on you, a /htui
// command, and a pane. It reads htui's state only by running `htui ... --json`
// (docs/htui-cli.md) and never writes anything of htui's.
import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, Timer } from 'claude-code'
import type { HtuiRow, HtuiWaiting } from '../types'
import { VERSION } from './version'

const waiting = atom({ plugin: 'htui', key: 'waiting' } as const, null as HtuiWaiting | null)
const skew = atom({ plugin: 'htui', key: 'skew' } as const, null as string | null)
const active = atom({ plugin: 'htui', key: 'active' } as const, false as boolean)

/** What one run of `htui` came to: its document, or the line that says why there is none. */
type Ran = { ok: true; doc: any } | { ok: false; line: string }

/** The pane's id, and the requestId its `ui.render` carries. */
const PANE = 'htui'

/** `42s`, `5m`, `3h`, `2d`: the largest whole unit, as htui's own text output. */
const age = (s: number): string =>
  s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)}m` : s < 86400 ? `${Math.floor(s / 3600)}h` : `${Math.floor(s / 86400)}d`

/** A row's reason as the overlay names it: `judge_failed` reads `judge failed`. */
const label = (reason: string): string => reason.replace(/_/g, ' ')

/** Runs `htui` and reads its JSON answer. Never throws: every failure is a line. */
async function runHtui($: EngineInterface, argv: readonly string[], timeoutMs: number): Promise<Ran> {
  const started = await $.clock.now()
  let r
  try {
    r = await $.process.run(argv, { timeoutMs })
  } catch {
    const elapsed = (await $.clock.now()) - started
    return { ok: false, line: elapsed >= timeoutMs ? 'htui: timed out' : 'htui: not found on PATH' }
  }
  if (r.stdout.trim() === '') return { ok: false, line: `htui printed nothing (exit ${r.exitCode})` }
  let doc: any
  try {
    doc = JSON.parse(r.stdout)
  } catch {
    doc = undefined
  }
  if (doc === null || typeof doc !== 'object' || Array.isArray(doc)) {
    return { ok: false, line: `htui: unreadable answer (exit ${r.exitCode})` }
  }
  if (typeof doc.version === 'number' && doc.version > 1) {
    return {
      ok: false,
      line: `htui: answer version ${doc.version} is newer than this plugin: run htui claude emit --force, then /reload-plugins`,
    }
  }
  if (typeof doc.error?.message === 'string') return { ok: false, line: `htui: ${doc.error.message}` }
  if (r.exitCode !== 0) return { ok: false, line: `htui: exit ${r.exitCode}` }
  return { ok: true, doc }
}

/** The rows of a `waiting` document; an entry that is not an object is left out. */
const rowsOf = (doc: any): HtuiRow[] => (Array.isArray(doc.rows) ? doc.rows.filter((r: unknown) => r !== null && typeof r === 'object') : [])

/** `/htui waiting`'s text, one line per row. */
const waitingText = (rows: readonly HtuiRow[]): string =>
  rows.length === 0
    ? 'Nothing waits on you.'
    : rows.map(r => `${r.item_key}  ${label(r.reason)}  ${r.step ?? '-'}  ${age(r.age_seconds)}  ${r.text}`).join('\n')

/** `auto`, `command` or `off` (D12), as `register` read it from the options. */
let panel: 'auto' | 'command' | 'off' = 'auto'
/** The poll period: `poll_seconds`, at least 5 (A-12). */
let periodMs = 30000
/** The poll timer while one runs; a module reload drops it. */
let timer: Timer | null = null
/** Set while a poll runs, so a slow poll is never overlapped by the next tick. */
let polling = false

/** One `htui waiting` poll into `waiting`; a failure replaces the rows, never keeps them. */
async function poll($: EngineInterface): Promise<void> {
  if (polling) return
  polling = true
  try {
    const ran = await runHtui($, ['htui', 'waiting', '--json'], 10000)
    const at = await $.clock.now()
    const next: HtuiWaiting = ran.ok
      ? { rows: rowsOf(ran.doc), workspace: ran.doc.workspace?.name ?? null, at, error: null }
      : { rows: [], workspace: null, at, error: ran.line }
    await update($, waiting, () => next)
  } catch {
    // The module unloaded mid-poll (its `$` is gone): nothing is left to show.
  } finally {
    polling = false
  }
}

/** One period: under `command`, stops the timer once the pane is closed; otherwise polls. */
async function tick($: EngineInterface): Promise<void> {
  if (panel === 'command') {
    let isOpen: boolean
    try {
      isOpen = (await $.ui.panes()).some(p => p.id === PANE)
    } catch {
      return
    }
    if (!isOpen) {
      timer?.cancel()
      timer = null
      return
    }
  }
  await poll($)
}

/** Starts the poll timer unless one runs. */
function startTimer($: EngineInterface): void {
  if (timer !== null) return
  timer = $.clock.every(periodMs, () => void tick($))
}

/** D14: names a binary whose version is not the plugin's. A failure leaves `skew` null; the poll reports it. */
async function checkVersion($: EngineInterface): Promise<void> {
  try {
    const r = await $.process.run(['htui', '--version'], { timeoutMs: 10000 })
    if (r.exitCode !== 0) return
    const bin = /^htui (\S+)/m.exec(r.stdout)?.[1]
    if (bin === undefined || bin === VERSION) return
    await update($, skew, () => `htui ${bin} ≠ plugin ${VERSION}: run htui claude emit --force, then /reload-plugins`)
  } catch {
    // Not on PATH or timed out: the poll reports that.
  }
}

export const register: Register = (on, options) => {
  panel = options.panel === 'command' || options.panel === 'off' ? options.panel : 'auto'
  periodMs = Math.max(5, Number(options.poll_seconds) || 30) * 1000
  timer = null
  polling = false

  on('session.start', async ($, e, next) => {
    if ((await $.env.get('HTUI_SESSION')) !== undefined) return next(e)
    await $.command.register({
      name: 'htui',
      description: 'htui: what waits on you, status, item search',
      argumentHint: '[status|waiting|search <query>]',
    })
    await update($, active, () => true)
    void checkVersion($)
    if (panel === 'auto') {
      void poll($)
      startTimer($)
    }
    return next(e)
  })

  on('command.run', { command: 'htui' }, async ($, e) => {
    const args = e.args.trim()
    const space = args.search(/\s/)
    const verb = space === -1 ? args : args.slice(0, space)
    const rest = space === -1 ? '' : args.slice(space).trim()

    if (verb === '' && panel !== 'off') {
      const o = await $.ui.open({ id: PANE, title: 'htui' })
      void poll($)
      startTimer($)
      return o.isPlaced ? {} : { text: `htui: the pane waits: ${o.reason}` }
    }
    if (verb === '' || verb === 'waiting') {
      const ran = await runHtui($, ['htui', 'waiting', '--json'], 10000)
      return { text: ran.ok ? waitingText(rowsOf(ran.doc)) : ran.line }
    }
    return { text: 'usage: /htui [status|waiting|search <query>]' }
  })
}

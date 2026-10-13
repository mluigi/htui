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

/**
 * `s` as a string with its control characters dropped, as htui's own text output drops them
 * (`state_cmd.rs`'s `clean`): stored text must not drive the terminal, nor break a one-line row.
 */
const clean = (s: unknown): string => String(s ?? '').replace(/\p{Cc}/gu, '')

/** A row's reason as the overlay names it: `judge_failed` reads `judge failed`. */
const label = (reason: string): string => reason.replace(/_/g, ' ')

/** Cuts `s` to `columns` characters, a trailing `…` marking the cut. */
const fit = (s: string, columns: number): string => {
  const chars = Array.from(s)
  return chars.length <= columns ? s : chars.slice(0, Math.max(0, columns - 1)).join('') + '…'
}

/** One pane row: reason, item, step and age in columns, then the text, cut to the pane's width. */
const rowLine = (r: HtuiRow, columns: number): string =>
  fit(
    `${label(r.reason).padEnd(13)}${r.item_key.padEnd(10)}${(r.step ?? '-').padEnd(16)}${age(r.age_seconds).padStart(4)}  ${r.text}`,
    columns,
  )

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
  if (typeof doc.error?.message === 'string') return { ok: false, line: `htui: ${clean(doc.error.message)}` }
  if (r.exitCode !== 0) return { ok: false, line: `htui: exit ${r.exitCode}` }
  return { ok: true, doc }
}

/**
 * The rows of a `waiting` document, reduced to the fields the mod reads and stripped of control
 * characters; an entry that is not an object is left out.
 */
const rowsOf = (doc: any): HtuiRow[] =>
  (Array.isArray(doc.rows) ? doc.rows : [])
    .filter((r: unknown) => r !== null && typeof r === 'object')
    .map((r: any) => ({
      reason: clean(r.reason),
      item_key: clean(r.item_key),
      step: typeof r.step === 'string' ? clean(r.step) : null,
      text: clean(r.text),
      age_seconds: Math.max(0, Math.floor(Number(r.age_seconds) || 0)),
    }))

/** `/htui waiting`'s text, one line per row. */
const waitingText = (rows: readonly HtuiRow[]): string =>
  rows.length === 0
    ? 'Nothing waits on you.'
    : rows.map(r => `${r.item_key}  ${label(r.reason)}  ${r.step ?? '-'}  ${age(r.age_seconds)}  ${r.text}`).join('\n')

/** `/htui status`'s text: the top bar's line, a note when permissions are unknown, then the skew line. */
async function statusText($: EngineInterface): Promise<string> {
  const ran = await runHtui($, ['htui', 'status', '--json'], 10000)
  const lines: string[] = []
  if (ran.ok) {
    const d = ran.doc
    lines.push(
      `${d.workspace ? clean(d.workspace.name) : 'no workspace'} · ${d.box ? clean(d.box.hostname) : 'no box'} · ${clean(d.store)} · ${d.working} working · ${d.waiting} waiting`,
    )
    if (d.permissions_known === false) lines.push('permission requests could not be listed; the count may be low')
  } else {
    lines.push(ran.line)
  }
  const s = await read($, skew)
  if (s !== null) lines.push(s)
  return lines.join('\n')
}

/** `/htui search <query>`'s text, one line per hit. The query rides inside one argument (A-6). */
async function searchText($: EngineInterface, query: string): Promise<string> {
  const ran = await runHtui($, ['htui', `--search-items=${query}`, '--json'], 60000)
  if (!ran.ok) return ran.line
  if (ran.doc.index === 'empty') return 'No concepts index yet: run htui --index-items.'
  const hits: any[] = Array.isArray(ran.doc.hits) ? ran.doc.hits.filter((h: unknown) => h !== null && typeof h === 'object') : []
  if (hits.length === 0) return 'No hits.'
  return hits
    .map(
      h =>
        `${clean(h.key)}  ${clean(h.place)}${h.document_kind ? ' ' + clean(h.document_kind) : ''}${h.resolution ? ' (' + clean(h.resolution) + ')' : ''}  ${clean(h.snippet)}`,
    )
    .join('\n')
}

/** `auto`, `command` or `off` (D12), as `register` read it from the options. */
let panel: 'auto' | 'command' | 'off' = 'auto'
/** The poll period: `poll_seconds` clamped to [5, 3600]; 0 or a non-number reads as 30 (A-12). */
let periodMs = 30000
/** `poll_seconds` as a period in seconds: 0 or a non-number is 30; else clamped to [5, 3600]. */
const pollSeconds = (v: unknown): number =>
  typeof v !== 'number' || Number.isNaN(v) || v === 0 ? 30 : Math.min(3600, Math.max(5, v))
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
      ? { rows: rowsOf(ran.doc), workspace: ran.doc.workspace ? clean(ran.doc.workspace.name) : null, at, error: null }
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

/**
 * D14: names a binary whose version is not the plugin's. Every run writes `skew`,
 * because `$.state` outlives a reload: after `emit --force` and `/reload-plugins`
 * the line an earlier check set must go. A failure writes null; the poll reports it.
 */
async function checkVersion($: EngineInterface): Promise<void> {
  let line: string | null = null
  try {
    const r = await $.process.run(['htui', '--version'], { timeoutMs: 10000 })
    const bin = r.exitCode === 0 ? /^htui (\S+)/m.exec(r.stdout)?.[1] : undefined
    if (bin !== undefined && bin !== VERSION) {
      line = `htui ${bin} ≠ plugin ${VERSION}: run htui claude emit --force, then /reload-plugins`
    }
  } catch {
    // Not on PATH or timed out: the poll reports that.
  }
  try {
    await update($, skew, () => line)
  } catch {
    // The module unloaded mid-check (its `$` is gone): nothing is left to show.
  }
}

export const register: Register = (on, options) => {
  panel = options.panel === 'command' || options.panel === 'off' ? options.panel : 'auto'
  periodMs = pollSeconds(options.poll_seconds) * 1000
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
    // D15: a session htui drives never registers /htui; should the command reach the hook anyway,
    // it runs nothing. Still no `next` (A-2): the hook answers its own command.
    if (!(await read($, active))) return { text: 'htui: stands down in sessions htui drives' }
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
    if (verb === 'status') return { text: await statusText($) }
    if (verb === 'search') {
      return { text: rest === '' ? 'usage: /htui search <query>' : await searchText($, rest) }
    }
    return { text: 'usage: /htui [status|waiting|search <query>]' }
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    if (panel !== 'auto' || e.props.hasSurvey || !(await read($, active))) return next(e)
    const w = await read($, waiting)
    const s = await read($, skew)
    const n = w?.rows.length ?? 0
    const error = w?.error ?? null
    if (n === 0 && error === null && s === null) return next(e)
    const { Box, Text } = $.ui.resolve(e)
    return (
      <Box key="htui-band" flexDirection="column">
        {n > 0 && <Text wrap="truncate">{`htui: ${n} waiting on you`}</Text>}
        {s !== null && <Text wrap="truncate">{s}</Text>}
        {error !== null && (
          <Text dimColor wrap="truncate">
            {error}
          </Text>
        )}
      </Box>
    )
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const w = await read($, waiting)
    const s = await read($, skew)
    const columns = e.props.bodyColumns
    const rows = w?.rows ?? []
    const { Box, Text } = $.ui.resolve(e)
    return (
      <Box key="htui-pane" flexDirection="column">
        <Text bold wrap="truncate">
          {w?.workspace ?? 'no workspace'} · {rows.length} waiting
        </Text>
        {s !== null && <Text wrap="truncate">{s}</Text>}
        {w?.error != null && (
          <Text dimColor wrap="truncate">
            {w.error}
          </Text>
        )}
        {rows.map((r, i) => (
          <Box key={`row-${i}`}>
            <Text wrap="truncate">{rowLine(r, columns)}</Text>
          </Box>
        ))}
        {w === null && <Text dimColor>htui: no answer yet</Text>}
        {w !== null && w.error === null && rows.length === 0 && <Text dimColor>Nothing waits on you.</Text>}
      </Box>
    )
  })
}

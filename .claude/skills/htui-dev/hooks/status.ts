// The status line's facts (TOOL-8 M2, docs/htui-dev.md): which item and HANDOFF phase, the
// branch's merge/ahead/dirty state, the repo disk and the dev Postgres, the last gate result.
// Pure functions over text, and `gather`, which reads the facts through an injected runner and
// reader so the kit tests drive it without an engine. Nothing here throws on bad input: a fact
// that cannot be read is left out of the line.

/** One process run: how it exited, or why it never did (not found, timed out). */
export type Ran = { exitCode: number; stdout: string; stderr: string } | { failed: string }
/** Runs `argv` in `cwd`; never throws. */
export type Runner = (argv: readonly string[], init: { cwd: string; timeoutMs: number }) => Promise<Ran>
/** A file's text, or null when it is absent or unreadable; never throws. */
export type Reader = (path: string) => Promise<string | null>
/** The checkout the mod serves: its root (as git prints it) and whether this is Windows. */
export type Host = { root: string; windows: boolean }

/** `root` and `parts` joined with `/`, which Windows accepts too; a trailing separator on `root` is dropped. */
export const join = (root: string, ...parts: string[]): string => [root.replace(/[\\/]+$/, ''), ...parts].join('/')

/** Whether a run exited, and with 0. */
const succeeded = (r: Ran): r is { exitCode: number; stdout: string; stderr: string } => !('failed' in r) && r.exitCode === 0

/** A JSON object: not null, not a list. */
const isObject = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object' && !Array.isArray(v)

/** `42s`, `5m`, `3h`, `2d`: the largest whole unit, as the htui plugin's own lines. */
export const age = (s: number): string =>
  s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)}m` : s < 86400 ? `${Math.floor(s / 3600)}h` : `${Math.floor(s / 86400)}d`

// ---- S1: the item ----

/**
 * The item the session works on: `HR_ITEM` when set, else the first `PREFIX-N` in the branch,
 * upper-cased, else the branch itself (`detached` for none). `key` is the HANDOFF ID, when there is one.
 */
export function itemOf(hrItem: string | undefined, branch: string | null): { label: string; key: string | null } {
  if (hrItem !== undefined && hrItem !== '') return { label: hrItem, key: hrItem }
  const m = branch === null ? null : /([A-Za-z]+-\d+)/.exec(branch)
  if (m !== null) {
    const key = m[1].toUpperCase()
    return { label: key, key }
  }
  return { label: branch ?? 'detached', key: null }
}

// ---- S2: the HANDOFF phase ----

/** What `phaseOf` reads: `<X> landed` from the last phase note, `open`, or `not open`. */
export type Phase = string

const escapeRegExp = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')

/** A checklist line of any item, open or closed: the end of the item before it. */
const ITEM_LINE = /^\s*[-*+]\s*\[[^\]]*\]\s*\*\*[A-Z]+-\d+/
const HEADING = /^#{1,6}\s/

/**
 * `key`'s phase from HANDOFF.md: the last `**Phase <X> landed` in its open checklist line and the
 * lines under it, up to the next item line or heading, as `<X> landed`; `open` with no phase note;
 * `not open` when the item has no open line.
 */
export function phaseOf(handoff: string, key: string): Phase {
  const lines = handoff.split(/\r?\n/)
  const open = new RegExp(`^- \\[ \\] \\*\\*${escapeRegExp(key)}(?![0-9])\\s*[-–—]`)
  const start = lines.findIndex(l => open.test(l))
  if (start === -1) return 'not open'
  let end = start + 1
  while (end < lines.length && !ITEM_LINE.test(lines[end]) && !HEADING.test(lines[end])) end++
  let phase: string | null = null
  for (const m of lines.slice(start, end).join('\n').matchAll(/\*\*Phase\s+(\S+?)\s+landed\b/g)) phase = m[1]
  return phase === null ? 'open' : `${phase} landed`
}

// ---- S4: the branch ----

export type Porcelain = { oid: string | null; head: string | null; ahead: number | null; dirty: number }

/**
 * `git status --porcelain=v2 --branch`: the commit, the branch (null when detached), the commits
 * ahead of the upstream (null with no upstream), and the entries that are changed, renamed,
 * unmerged or untracked.
 */
export function parsePorcelainV2(stdout: string): Porcelain {
  const p: Porcelain = { oid: null, head: null, ahead: null, dirty: 0 }
  for (const line of stdout.split(/\r?\n/)) {
    let m: RegExpExecArray | null
    if ((m = /^# branch\.oid (\S+)/.exec(line)) !== null) p.oid = m[1] === '(initial)' ? null : m[1]
    else if ((m = /^# branch\.head (.+)$/.exec(line)) !== null) p.head = m[1] === '(detached)' ? null : m[1]
    else if ((m = /^# branch\.ab \+(\d+) -\d+/.exec(line)) !== null) p.ahead = Number(m[1])
    else if (/^[12u?] /.test(line)) p.dirty++
  }
  return p
}

// ---- S5/S6: dev-health ----

export type PgState = 'up' | 'recovering' | 'down' | 'unknown'
export type Health = { usedPct: number | null; pg: PgState | null }

const PG_STATES: readonly string[] = ['up', 'recovering', 'down', 'unknown']

/** scripts/dev-health's JSON line; anything it cannot read is null. */
export function parseHealth(stdout: string): Health {
  let doc: unknown
  try {
    doc = JSON.parse(stdout)
  } catch {
    return { usedPct: null, pg: null }
  }
  if (!isObject(doc)) return { usedPct: null, pg: null }
  const pct = isObject(doc.disk) ? doc.disk.used_pct : undefined
  const state = isObject(doc.postgres) ? doc.postgres.state : undefined
  return {
    usedPct: typeof pct === 'number' && Number.isFinite(pct) ? pct : null,
    pg: typeof state === 'string' && PG_STATES.includes(state) ? (state as PgState) : null,
  }
}

// ---- S7: the gate ----

export type GateStep = { name: string; status: string; seconds: number }
export type GateDoc = {
  head: string
  started: string
  finished: string | null
  state: 'running' | 'passed' | 'failed'
  steps: GateStep[]
}
/** A parsed `last.json`: the document, null when there is none, or `unreadable`. */
export type GateRead = GateDoc | null | 'unreadable'

/** `target/gate/last.json` as gate.sh/.ps1 writes it; null text is no file. */
export function parseGate(text: string | null): GateRead {
  if (text === null) return null
  let doc: unknown
  try {
    doc = JSON.parse(text)
  } catch {
    return 'unreadable'
  }
  if (!isObject(doc)) return 'unreadable'
  const { head, started, finished, state, steps } = doc
  if (state !== 'running' && state !== 'passed' && state !== 'failed') return 'unreadable'
  if (!Array.isArray(steps)) return 'unreadable'
  return {
    head: typeof head === 'string' ? head : '',
    started: typeof started === 'string' ? started : '',
    finished: typeof finished === 'string' ? finished : null,
    state,
    steps: steps.filter(isObject).map(s => ({
      name: String(s.name ?? ''),
      status: String(s.status ?? ''),
      seconds: Math.max(0, Math.floor(Number(s.seconds) || 0)),
    })),
  }
}

/** ` 4m` for the seconds from `isoTime` to `now`, or nothing when the time does not parse. */
const since = (isoTime: string | null, now: number): string => {
  const t = isoTime === null ? NaN : Date.parse(isoTime)
  return Number.isNaN(t) ? '' : ` ${age(Math.max(0, Math.floor((now - t) / 1000)))}`
}

const failedSteps = (doc: GateDoc): string[] => doc.steps.filter(s => s.status === 'failed').map(s => s.name)

/** The outcome alone: `gate passed`, `gate failed: test, abort-grep`, or `gate running`. */
const outcome = (doc: GateDoc): string => {
  if (doc.state !== 'failed') return `gate ${doc.state}`
  const failed = failedSteps(doc)
  return failed.length === 0 ? 'gate failed' : `gate failed: ${failed.join(', ')}`
}

/**
 * The gate's part of the line: `gate running 4m` (from its start), `gate passed 2h` or `gate
 * failed: test 40m` (from its end), `(older HEAD)` when it ran on another commit; null with no file.
 */
export function gateSummary(doc: GateRead, head: string | null, now: number): string | null {
  if (doc === null) return null
  if (doc === 'unreadable') return 'gate: last.json unreadable'
  const when = since(doc.state === 'running' ? doc.started : doc.finished, now)
  const older = head !== null && doc.head !== '' && doc.head !== head ? ' (older HEAD)' : ''
  return `${outcome(doc)}${when}${older}`
}

/** The toast for a finished gate; null while it runs. */
export const gateToast = (doc: GateDoc): string | null => (doc.state === 'running' ? null : outcome(doc))

/** How long a `running` result blocks `/gate`: a gate killed mid-run leaves one behind. */
export const FRESH_MS = 4 * 3600_000

/** Whether a gate is running and started under FRESH_MS ago. */
export function isFreshRun(doc: GateRead, now: number): boolean {
  if (doc === null || doc === 'unreadable' || doc.state !== 'running') return false
  const t = Date.parse(doc.started)
  return Number.isNaN(t) || now - t < FRESH_MS
}

/** `/gate status`: the summary, then one line per step. */
export function gateStatusText(doc: GateRead, head: string | null, now: number): string {
  const summary = gateSummary(doc, head, now)
  if (summary === null) return 'no gate result yet'
  if (doc === null || doc === 'unreadable') return summary
  return [summary, ...doc.steps.map(s => `${s.name} ${s.status} ${age(s.seconds)}`)].join('\n')
}

// ---- the line ----

/** Every fact the line shows; null where it could not be read. */
export type Snapshot = {
  item: { label: string; key: string | null }
  phase: Phase | null
  merging: boolean
  ahead: number | null
  dirty: number | null
  health: Health
  gate: GateRead
  head: string | null
}

/** The disk use above which the line shows it (ANA-29 D6). */
const DISK_WARN_PCT = 90

/**
 * The ambient line, quiet parts dropped: `TOOL-8 M1 landed · merging · 2 ahead · 1 dirty ·
 * disk 93% · pg down · gate passed 2h`.
 */
export function composeAmbient(s: Snapshot, now: number): string {
  const parts = [s.phase === null ? s.item.label : `${s.item.label} ${s.phase}`]
  if (s.merging) parts.push('merging')
  if (s.ahead !== null && s.ahead > 0) parts.push(`${s.ahead} ahead`)
  if (s.dirty !== null && s.dirty > 0) parts.push(`${s.dirty} dirty`)
  if (s.health.usedPct !== null && s.health.usedPct > DISK_WARN_PCT) parts.push(`disk ${s.health.usedPct}%`)
  if (s.health.pg === 'recovering' || s.health.pg === 'down') parts.push(`pg ${s.health.pg}`)
  const gate = gateSummary(s.gate, s.head, now)
  if (gate !== null) parts.push(gate)
  return parts.join(' · ')
}

/** The plugin's one status line: a guard warning leads the ambient line; neither clears it. */
export function composeLine(warning: string | null, ambient: string | null): string | undefined {
  if (warning !== null && ambient !== null) return `⚠ ${warning} · ${ambient}`
  return warning ?? ambient ?? undefined
}

// ---- gathering ----

const GIT_MS = 10_000
const HEALTH_MS = 20_000

/**
 * Reads every fact at once. `healthArgv` runs scripts/dev-health for the platform. A piece that
 * fails is null; the item falls back to `?` when git could not say which branch this is.
 */
export async function gather(host: Host, hrItem: string | undefined, run: Runner, read: Reader, healthArgv: readonly string[]): Promise<Snapshot> {
  const init = { cwd: host.root, timeoutMs: GIT_MS }
  const [status, merge, health, handoff, gateText] = await Promise.all([
    run(['git', '--no-optional-locks', 'status', '--porcelain=v2', '--branch'], init),
    run(['git', 'rev-parse', '-q', '--verify', 'MERGE_HEAD'], init),
    run(healthArgv, { cwd: host.root, timeoutMs: HEALTH_MS }),
    read(join(host.root, 'HANDOFF.md')),
    read(join(host.root, 'target', 'gate', 'last.json')),
  ])
  const porcelain = succeeded(status) ? parsePorcelainV2(status.stdout) : null
  let ahead = porcelain?.ahead ?? null
  if (porcelain !== null && ahead === null && porcelain.head !== null && porcelain.head !== 'main') {
    const count = await run(['git', 'rev-list', '--count', 'main..HEAD'], init)
    const n = succeeded(count) ? Number(count.stdout.trim()) : NaN
    ahead = Number.isSafeInteger(n) ? n : null
  }
  const item =
    porcelain === null && (hrItem === undefined || hrItem === '') ? { label: '?', key: null } : itemOf(hrItem, porcelain?.head ?? null)
  return {
    item,
    phase: item.key !== null && handoff !== null ? phaseOf(handoff, item.key) : null,
    merging: succeeded(merge),
    ahead,
    dirty: porcelain?.dirty ?? null,
    health: succeeded(health) ? parseHealth(health.stdout) : { usedPct: null, pg: null },
    gate: parseGate(gateText),
    head: porcelain?.oid ?? null,
  }
}

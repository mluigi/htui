// Shared helpers for the mod's kit tests (`claude plugin test`). The JSON
// documents are docs/htui-cli.md's own examples.
import type { On } from 'claude-code'
import type { Engine, MockClock } from 'claude-code/testing'
import { mock } from 'claude-code/testing'

/** A `process.run` answer: the child exited with `exitCode` after writing `stdout`. */
export const ok = (stdout: string, exitCode = 0) => ({
  value: { exitCode, stdout, stderr: '', isStdoutTruncated: false, isStderrTruncated: false },
})

/** How the fake `htui` answers one verb: a finished run, a spawn failure, or a run that outlasts its bound. */
export type Answer = ReturnType<typeof ok> | 'deny' | 'hang'

/** One `$.process.run` the mod made. */
export type Call = { argv: string[]; timeoutMs?: number }

/** The verb a call names: `waiting`, `status`, `--version`, or `search` for `--search-items=`. */
const verb = (argv: readonly string[]): string =>
  (argv[1] ?? '').startsWith('--search-items=') ? 'search' : (argv[1] ?? '')

/**
 * Stands in for the `htui` binary beneath the mod. Records every call in
 * `calls`; a verb missing from `answers` is answered like a missing binary.
 * `'hang'` needs `clock`: it answers `{ deny }` once the test advances it 10 s.
 */
export const htui = (on: On, answers: Record<string, Answer>, calls: Call[], clock?: MockClock) => {
  on('process.run', async ($, e) => {
    calls.push({ argv: [...e.argv], timeoutMs: e.init?.timeoutMs })
    const answer = answers[verb(e.argv)] ?? 'deny'
    if (answer === 'deny') return { deny: 'spawn htui ENOENT' }
    if (answer === 'hang') {
      await clock!.sleep(10000)
      return { deny: 'timed out' }
    }
    return answer as never
  })
}

/** The `waiting` calls among `calls`. */
export const waitingCalls = (calls: Call[]) => calls.filter(c => c.argv[1] === 'waiting').length

/** Raises `session.start` the way a session does, with `env` as the process environment. */
export const start = async ($: Engine, on: On, env: Record<string, string> = {}) => {
  mock.env(on, env)
  on('command.register', async () => ({ value: undefined }))
  on('session.start', async ($, e) => e)
  await $.session.start({ source: 'startup', cwd: '/tmp' } as never)
}

/** The engine's own drawing beneath `component` (A-3): the kit has none, so a passing hook needs this. */
export const engineBottom = (on: On, component: 'AbovePrompt' | 'Pane') => {
  on('ui.render', { component }, async ($, e) => {
    const { Box } = $.ui.resolve(e)
    return <Box key="engine" />
  })
}

export const BAND = { hasSurvey: false, isWorking: false, maxRows: 10, bodyColumns: 80 } as never
export const PANE = { title: 'htui', isFocused: false, bodyColumns: 60, placement: 'inline' } as never

const WORKSPACE = { id: '00000000-0000-0000-0000-000000000001', slug: 'platform', name: 'Platform' }
const PROJECTS = [
  { id: '00000000-0000-0000-0000-000000000010', slug: 'htui', name: 'htui' },
  { id: '00000000-0000-0000-0000-000000000011', slug: 'agy', name: 'Antigravity' },
]

export const STATUS = JSON.stringify({
  version: 1,
  kind: 'status',
  store: 'online',
  workspace: WORKSPACE,
  projects: PROJECTS,
  box: { id: '00000000-0000-0000-0000-0000000000b0', hostname: 'desk', registered: true },
  working: 1,
  waiting: 2,
  permissions_known: true,
})

export const WAITING = JSON.stringify({
  version: 1,
  kind: 'waiting',
  store: 'online',
  workspace: WORKSPACE,
  projects: PROJECTS,
  working: 1,
  permissions_known: true,
  rows: [
    {
      reason: 'gate',
      item_key: 'MOD-4',
      item_id: '00000000-0000-0000-0000-000000000101',
      run_id: '00000000-0000-0000-0000-000000000201',
      step_id: '00000000-0000-0000-0000-000000000301',
      step: 'prd 0.1',
      text: 'approve the PRD',
      since: '2026-09-03T06:00:00Z',
      age_seconds: 64800,
    },
    {
      reason: 'unblock',
      item_key: 'MOD-7',
      item_id: '00000000-0000-0000-0000-000000000104',
      run_id: null,
      step_id: null,
      step: null,
      text: 'blocked, no active run: u reopens it',
      since: '2026-09-02T00:00:00Z',
      age_seconds: 172800,
    },
  ],
})

export const WAITING_NONE = JSON.stringify({
  version: 1,
  kind: 'waiting',
  store: 'online',
  workspace: WORKSPACE,
  projects: PROJECTS,
  working: 0,
  permissions_known: true,
  rows: [],
})

export const OFFLINE =
  '{"version":1,"kind":"status","store":"offline","error":{"code":"unreachable","message":"cannot reach Postgres: store unreachable: connection refused"}}'

export const NO_DSN =
  '{"version":1,"kind":"waiting","store":"refused","error":{"code":"no_dsn","message":"no Postgres DSN is stored; run `htui --set-dsn` first"}}'

export const SEARCH = JSON.stringify({
  version: 1,
  kind: 'search',
  store: 'online',
  index: 'online',
  query: 'closed items',
  decisions: false,
  hits: [
    {
      key: 'ANA-11',
      place: 'item',
      document_kind: null,
      resolution: null,
      requirement_state: null,
      score: 0.731,
      snippet: 'a decision is a closed item',
    },
    {
      key: 'MOD-34',
      place: 'document',
      document_kind: 'prd',
      resolution: 'done',
      requirement_state: null,
      score: 0.5,
      snippet: 'the concepts index',
    },
  ],
})

export const SEARCH_EMPTY = JSON.stringify({
  version: 1,
  kind: 'search',
  store: 'online',
  index: 'empty',
  query: 'closed items',
  decisions: false,
  hits: [],
})

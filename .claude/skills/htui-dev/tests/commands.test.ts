import { test, expect } from 'claude-code/testing'
import { gateArgv, gateCommand, GATE_STEPS, scriptArgv, scriptCommand } from '../hooks/commands'
import type { Ran } from '../hooks/status'

const HEAD = 'a'.repeat(40)
const T0 = Date.parse('2026-10-14T12:00:00Z')
const iso = (ms: number) => new Date(ms).toISOString().replace(/\.\d{3}Z$/, 'Z')
const POSIX = { root: '/repo', windows: false }
const WIN = { root: 'C:/r/', windows: true }
const LAST = '/repo/target/gate/last.json'

const running = (startedAgo: number) =>
  JSON.stringify({ head: HEAD, started: iso(T0 - startedAgo * 1000), finished: null, state: 'running', steps: [] })

type Call = { argv: string[]; cwd: string; timeoutMs: number }

/** A runner that answers every call with `ran`, recording it. */
const runner = (calls: Call[], ran: Ran = { exitCode: 0, stdout: 'gate started, pid 42\n', stderr: '' }) =>
  async (argv: readonly string[], init: { cwd: string; timeoutMs: number }) => {
    calls.push({ argv: [...argv], ...init })
    return ran
  }
const files = (map: Record<string, string>) => async (p: string) => map[p] ?? null

test('POSIX runs each script with bash and the repo root', () => {
  expect(scriptArgv(POSIX, 'next')).toEqual([
    'bash',
    '/repo/.claude/skills/handoff-run/scripts/next-item.sh',
    '--repo-root',
    '/repo',
  ])
  expect(scriptArgv(POSIX, 'validate')).toEqual([
    'bash',
    '/repo/.claude/skills/handoff-run/scripts/validate-workflow-docs.sh',
    '--repo-root',
    '/repo',
  ])
  expect(scriptArgv(POSIX, 'health')).toEqual(['bash', '/repo/scripts/dev-health.sh'])
  expect(gateArgv(POSIX, null)).toEqual(['bash', '/repo/scripts/gate.sh', '--detach'])
  expect(gateArgv(POSIX, 'test')).toEqual(['bash', '/repo/scripts/gate.sh', '--detach', '--only', 'test'])
})

test('Windows runs each .ps1 through pwsh, joining paths with /', () => {
  expect(scriptArgv(WIN, 'next')).toEqual([
    'pwsh',
    '-NoProfile',
    '-File',
    'C:/r/.claude/skills/handoff-run/scripts/next-item.ps1',
    '-RepoRoot',
    'C:/r/',
  ])
  expect(scriptArgv(WIN, 'health')).toEqual(['pwsh', '-NoProfile', '-File', 'C:/r/scripts/dev-health.ps1'])
  expect(gateArgv(WIN, 'fmt')).toEqual(['pwsh', '-NoProfile', '-File', 'C:/r/scripts/gate.ps1', '-Detach', '-Only', 'fmt'])
})

test("the step list is gate.sh's", () => {
  expect([...GATE_STEPS]).toEqual(['fmt', 'clippy-all', 'clippy-bare', 'test', 'abort-grep', 'migrations', 'workflow-docs', 'plugin'])
})

test("a bare /gate starts the detached gate in the repo root and answers the launcher's line", async () => {
  const calls: Call[] = []
  const got = await gateCommand('', POSIX, runner(calls), files({}), T0, HEAD)
  expect(got).toEqual({ text: 'gate started, pid 42', exitCode: 0 })
  expect(calls).toEqual([{ argv: ['bash', '/repo/scripts/gate.sh', '--detach'], cwd: '/repo', timeoutMs: 30000 }])
})

test('/gate refuses while a gate started under 4 h ago is running, and runs nothing', async () => {
  const calls: Call[] = []
  const got = await gateCommand('', POSIX, runner(calls), files({ [LAST]: running(720) }), T0, HEAD)
  expect(got).toEqual({
    text: 'gate already running (started 12m ago): /gate status for progress, /gate force to start another',
    exitCode: 1,
  })
  expect(calls).toEqual([])
})

test('/gate starts over a running result older than 4 h', async () => {
  const calls: Call[] = []
  await gateCommand('', POSIX, runner(calls), files({ [LAST]: running(4 * 3600 + 5) }), T0, HEAD)
  expect(calls.length).toBe(1)
})

test('/gate force starts despite a fresh running result', async () => {
  const calls: Call[] = []
  await gateCommand('force', POSIX, runner(calls), files({ [LAST]: running(60) }), T0, HEAD)
  expect(calls.map(c => c.argv)).toEqual([['bash', '/repo/scripts/gate.sh', '--detach']])
})

test('/gate <step> runs that step alone, and is refused while one runs', async () => {
  const calls: Call[] = []
  await gateCommand(' test ', POSIX, runner(calls), files({}), T0, HEAD)
  expect(calls.map(c => c.argv)).toEqual([['bash', '/repo/scripts/gate.sh', '--detach', '--only', 'test']])
  const refused = await gateCommand('test', POSIX, runner(calls), files({ [LAST]: running(60) }), T0, HEAD)
  expect(refused.exitCode).toBe(1)
  expect(calls.length).toBe(1)
})

test('/gate status prints the summary and each step', async () => {
  const doc = JSON.stringify({
    head: HEAD,
    started: iso(T0 - 999_000),
    finished: iso(T0 - 60_000),
    state: 'failed',
    steps: [
      { name: 'fmt', status: 'passed', seconds: 3 },
      { name: 'test', status: 'failed', seconds: 125 },
    ],
  })
  const calls: Call[] = []
  expect(await gateCommand('status', POSIX, runner(calls), files({ [LAST]: doc }), T0, HEAD)).toEqual({
    text: 'gate failed: test 1m\nfmt passed 3s\ntest failed 2m',
    exitCode: 0,
  })
  expect(await gateCommand('status', POSIX, runner(calls), files({}), T0, HEAD)).toEqual({
    text: 'no gate result yet',
    exitCode: 1,
  })
  expect(calls).toEqual([])
})

test('/gate with an unknown word lists the steps and runs nothing', async () => {
  const calls: Call[] = []
  expect(await gateCommand('bogus', POSIX, runner(calls), files({}), T0, HEAD)).toEqual({
    text: 'unknown step "bogus" - /gate [step|status|force]; steps: fmt, clippy-all, clippy-bare, test, abort-grep, migrations, workflow-docs, plugin',
    exitCode: 2,
  })
  expect(calls).toEqual([])
})

test('/next answers the script stdout and its exit code', async () => {
  const calls: Call[] = []
  const got = await scriptCommand('next', POSIX, runner(calls, { exitCode: 0, stdout: 'next-item pre-filter\n  1. TOOL-8\n', stderr: 'noise' }))
  expect(got).toEqual({ text: 'next-item pre-filter\n  1. TOOL-8', exitCode: 0 })
  expect(calls[0].cwd).toBe('/repo')
  expect(calls[0].timeoutMs).toBe(60000)
})

test('a failing script appends its stderr and passes its code', async () => {
  const got = await scriptCommand('next', POSIX, runner([], { exitCode: 2, stdout: 'partial\n', stderr: 'HANDOFF.md missing\n' }))
  expect(got).toEqual({ text: 'partial\nHANDOFF.md missing', exitCode: 2 })
})

test('/validate answers the validator output and exit 1 on findings', async () => {
  const out = '[ERROR] HANDOFF.md:3 bad line\nworkflow-docs validation: 1 error(s), 0 warning(s)\n'
  const got = await scriptCommand('validate', POSIX, runner([], { exitCode: 1, stdout: out, stderr: '' }))
  expect(got).toEqual({ text: out.trimEnd(), exitCode: 1 })
})

test('a script with no output says so', async () => {
  expect(await scriptCommand('validate', POSIX, runner([], { exitCode: 0, stdout: '', stderr: '' }))).toEqual({
    text: '(validate-workflow-docs: no output, exit 0)',
    exitCode: 0,
  })
})

test('a script that cannot start answers why, with exit 1', async () => {
  expect(await scriptCommand('next', POSIX, runner([], { failed: 'not found' }))).toEqual({
    text: 'htui-dev: next-item could not run: not found',
    exitCode: 1,
  })
  expect(await gateCommand('', POSIX, runner([], { failed: 'timed out' }), files({}), T0, HEAD)).toEqual({
    text: 'htui-dev: gate could not run: timed out',
    exitCode: 1,
  })
})

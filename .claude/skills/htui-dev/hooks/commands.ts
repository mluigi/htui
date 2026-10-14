// The turnless commands (TOOL-8 M2, docs/htui-dev.md): `/gate` starts TOOL-9's gate detached,
// `/next` and `/validate` run the workflow scripts. Each runs the `.ps1` through pwsh on Windows
// and the `.sh` through bash elsewhere (ANA-29 §6.3), and answers the script's own text and exit
// code. The runner and reader are injected, so the kit tests drive these without an engine.
import { age, gateStatusText, isFreshRun, join, parseGate, type Host, type Ran, type Reader, type Runner } from './status'

/** gate.sh's STEPS (and gate.ps1's ValidateSet), in lockstep. */
export const GATE_STEPS = ['fmt', 'clippy-all', 'clippy-bare', 'postgres', 'test', 'abort-grep', 'migrations', 'workflow-docs', 'plugin'] as const

/** What a command answers: its transcript text, and the exit code a `claude -p "/cmd"` run takes. */
export type Answer = { text: string; exitCode: number }

export type Script = 'next' | 'validate' | 'health'

/** Each script's path from the repo root, without its `.sh`/`.ps1`. */
const SCRIPTS: Record<Script, string> = {
  next: '.claude/skills/handoff-run/scripts/next-item',
  validate: '.claude/skills/handoff-run/scripts/validate-workflow-docs',
  health: 'scripts/dev-health',
}

/** How a script is started on this platform: `bash x.sh`, or `pwsh -NoProfile -File x.ps1`. */
const launch = (h: Host, path: string): string[] =>
  h.windows ? ['pwsh', '-NoProfile', '-File', join(h.root, `${path}.ps1`)] : ['bash', join(h.root, `${path}.sh`)]

/** A workflow script's argv; `next` and `validate` are told the repo root. */
export function scriptArgv(h: Host, s: Script): string[] {
  const root = s === 'health' ? [] : h.windows ? ['-RepoRoot', h.root] : ['--repo-root', h.root]
  return [...launch(h, SCRIPTS[s]), ...root]
}

/** The detached gate's argv, for one step or all. */
export function gateArgv(h: Host, step: string | null): string[] {
  const only = step === null ? [] : h.windows ? ['-Only', step] : ['--only', step]
  return [...launch(h, 'scripts/gate'), h.windows ? '-Detach' : '--detach', ...only]
}

/** A run as a command's answer: stdout, with stderr after it on a failure, and the exit code. */
export function answerOf(ran: Ran, what: string): Answer {
  if ('failed' in ran) return { text: `htui-dev: ${what} could not run: ${ran.failed}`, exitCode: 1 }
  let text = ran.stdout.trimEnd()
  if (ran.exitCode !== 0 && ran.stderr.trim() !== '') text = text === '' ? ran.stderr.trimEnd() : `${text}\n${ran.stderr.trimEnd()}`
  if (text === '') text = `(${what}: no output, exit ${ran.exitCode})`
  return { text, exitCode: exitCodeOf(ran.exitCode) }
}

/** An exit code `command.run` takes (a whole number 0-255; any other fails the hook): else 1. */
const exitCodeOf = (c: number): number => (Number.isInteger(c) && c >= 0 && c <= 255 ? c : 1)

/** gate.sh --detach returns at once; the bound only covers a launcher that hangs. */
const GATE_LAUNCH_MS = 30_000
/** next-item and the validator finish in about a second here. */
const SCRIPT_MS = 60_000

/**
 * `/gate [step|status|force]`. Bare or with a step: refuses while a gate started under 4 h ago is
 * running, else starts it detached. `force` starts the whole gate regardless. `status` prints the
 * last result. Exit codes follow gate.sh: 1 refused or no result, 2 usage.
 */
export async function gateCommand(args: string, h: Host, run: Runner, read: Reader, now: number, head: string | null): Promise<Answer> {
  const verb = args.trim().split(/\s+/)[0] ?? ''
  const doc = parseGate(await read(join(h.root, 'target', 'gate', 'last.json')))
  if (verb === 'status') return { text: gateStatusText(doc, head, now), exitCode: doc === null || doc === 'unreadable' ? 1 : 0 }
  const step = verb === '' || verb === 'force' ? null : verb
  if (step !== null && !(GATE_STEPS as readonly string[]).includes(step)) {
    return { text: `unknown step "${step}" - /gate [step|status|force]; steps: ${GATE_STEPS.join(', ')}`, exitCode: 2 }
  }
  if (verb !== 'force' && isFreshRun(doc, now) && doc !== null && doc !== 'unreadable') {
    const t = Date.parse(doc.started)
    const ago = Number.isNaN(t) ? '' : ` (started ${age(Math.max(0, Math.floor((now - t) / 1000)))} ago)`
    return { text: `gate already running${ago}: /gate status for progress, /gate force to start another`, exitCode: 1 }
  }
  return answerOf(await run(gateArgv(h, step), { cwd: h.root, timeoutMs: GATE_LAUNCH_MS }), 'gate')
}

/** `/next` and `/validate`: the script's text and exit code. */
export async function scriptCommand(s: 'next' | 'validate', h: Host, run: Runner): Promise<Answer> {
  return answerOf(await run(scriptArgv(h, s), { cwd: h.root, timeoutMs: SCRIPT_MS }), s === 'next' ? 'next-item' : 'validate-workflow-docs')
}

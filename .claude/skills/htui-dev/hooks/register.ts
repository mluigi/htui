// htui-dev: developing htui. Guards that each refuse, with the fix in its reason, a kind of
// call that has cost this repo real work (docs/htui-dev.md):
//   G1  cargo test that builds feature-gated integration tests without their feature (0 tests, green)
//   G2  cargo test with the other crate's test feature name (testkit vs test-support)
//   G3  the literal extensibility.rs forbids, written into crates/**/*.{rs,json} by an edit tool
//   G4  file writes, and the git writes docs/htui-dev.md lists, from an agent started read-only
//   G5  a subagent asked to run on Fable
//   G6  git commit --no-verify (or -n), which skips the repo's commit hooks
// Every guard is a `{ deny }`, never a rewrite, and judges before `next`. A guard that throws
// lets the call through and says so on the status line (ANA-29 §6.3, "fail open"): the guards
// are a safety net, not a sandbox. Beside them (M2): one status line, refreshed by a 60 s timer in
// interactive sessions (item and HANDOFF phase, merge/ahead/dirty, disk, Postgres, last gate), a
// toast when a gate finishes, and the turnless /gate, /next and /validate. The mod runs git and
// the repo's scripts, reads HANDOFF.md and target/gate/last.json, writes no file and reaches no
// network. Its values in `$.state` (read-only agents' ids, the gate last toasted, whether the
// session runs the timer) survive a reload; the timer does not, and a tool call re-arms it.
import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, Timer } from 'claude-code'
import type { HtuiDevGateSeen, HtuiDevReadOnly, HtuiDevReadOnlyCalls } from '../types'
import { gateCommand, scriptArgv, scriptCommand, type Answer } from './commands'
import {
  composeAmbient,
  composeLine,
  gateToast,
  gather,
  join,
  parseGate,
  type GateRead,
  type Host,
  type Ran,
  type Reader,
  type Runner,
} from './status'
import {
  cargoTest,
  coarseGitWrite,
  type Cwd,
  declaresReadOnly,
  fableModel,
  forbiddenLiteral,
  gitWrite,
  isFable,
  movedTo,
  noVerify,
  readOnlyEdit,
  readOnlyGit,
  writesNothing,
} from './guards'
import { simpleCommands, type Words } from './shell'

const readOnly = atom({ plugin: 'htui-dev', key: 'readOnly' } as const, [] as HtuiDevReadOnly)
const readOnlyCalls = atom({ plugin: 'htui-dev', key: 'readOnlyCalls' } as const, [] as HtuiDevReadOnlyCalls)
const gateSeen = atom({ plugin: 'htui-dev', key: 'gateSeen' } as const, null as HtuiDevGateSeen | null)
const ambient = atom({ plugin: 'htui-dev', key: 'ambient' } as const, false as boolean)

/** The status timer's period (ANA-29 §6.1). */
const PERIOD_MS = 60_000
/** How long a guard warning leads the line, from the first tick that showed it. */
const WARNING_MS = 10 * 60_000

/** `status_line` as `register` read it; off stops the timer. */
let statusLine = true
/** The status timer while one runs; a module reload drops it. */
let timer: Timer | null = null
/** Set while a tick runs, so a slow tick is never overlapped by the next. */
let polling = false
/** Whether a tool call already asked `ambient` since this module loaded. */
let rearmChecked = false
/** The last guard warning, and the tick time it was first shown at (null until a tick stamps it). */
let warning: { text: string; at: number | null } | null = null
/** The last ambient line a tick composed. */
let ambientLine: string | null = null
/** The last line put on the status line, so an unchanged tick sends nothing. */
let shown: string | undefined
/** The checkout, found once per module load. */
let hostMemo: Host | null = null

/** Puts the composed line on the plugin's status line (`force`: even when unchanged); never throws. */
function show($: EngineInterface, force = false): void {
  const line = composeLine(warning?.text ?? null, ambientLine)
  if (!force && line === shown) return
  shown = line
  try {
    // Fire and forget: the call answers nothing, so only a throw (a re-entry) is seen.
    $.ui.status(line)
  } catch {
    // The next tick sends the line again.
    shown = undefined
  }
}

/**
 * Puts `text` on the plugin's status line, ahead of the ambient line while there is one; never
 * throws and calls nothing on `$` but the status line (it runs where every other `$` call rejects).
 */
function warn($: EngineInterface, text: string): void {
  warning = { text, at: null }
  show($, true)
}

/** Runs one guard; a throw is a status warning and no refusal (ANA-29 §6.3). */
function judge($: EngineInterface, guard: string, what: 'call' | 'spawn', decide: () => string | null): string | null {
  try {
    return decide()
  } catch {
    warn($, `htui-dev: ${guard} guard failed, ${what} allowed`)
    return null
  }
}

/**
 * Whether the list `load` reads holds `id`; an undefined or empty `id` reads nothing. A read that
 * fails, or a value that is no list, warns with `failed` and answers false.
 */
async function holds($: EngineInterface, load: () => Promise<unknown>, id: string | undefined, failed: string): Promise<boolean> {
  if (id === undefined || id === '') return false
  try {
    const ids = await load()
    if (!Array.isArray(ids)) throw new TypeError('the state holds no list')
    return ids.includes(id)
  } catch {
    warn($, failed)
    return false
  }
}

/** Whether `agentId` was tagged read-only. The main loop (no `agentId`) never reads the state. */
const isTagged = ($: EngineInterface, agentId: string | undefined, failed: string): Promise<boolean> =>
  holds($, () => read($, readOnly), agentId, failed)

/** Whether a read-only agent made the Workflow or Agent call `toolUseId`. */
const isReadOnlyCall = ($: EngineInterface, toolUseId: string | undefined, failed: string): Promise<boolean> =>
  holds($, () => read($, readOnlyCalls), toolUseId, failed)

/** The list with `id` added, once. */
const adding =
  (id: string) =>
  (ids: readonly string[] | undefined): readonly string[] =>
    (ids ?? []).includes(id) ? ids! : [...(ids ?? []), id]

/** Runs a state write; one that fails warns with `failed`. */
async function store($: EngineInterface, write: () => Promise<unknown>, failed: string): Promise<void> {
  try {
    await write()
  } catch {
    warn($, failed)
  }
}

/**
 * `agent.spawn`'s backstop (a throw, an overrun budget, a wrong answer shape): a spawn is never
 * refused here. A settled `next` is replayed, with a warning that the agent may have started before
 * its read-only tag was written; otherwise the spawn goes ahead, with a warning.
 */
export function spawnBackstop<E, R>($: EngineInterface, e: E, next: ((e: E) => R) & { readonly called: boolean }): R {
  if (next.called) {
    warn($, 'htui-dev: agent.spawn guard failed after the spawn, read-only tag may be missing')
    return next(e)
  }
  warn($, 'htui-dev: agent.spawn guard failed, spawn allowed')
  return next(e)
}

// ---- M2: the status line ----

/** The clock's time; 0 when it cannot be read. */
async function nowOf($: EngineInterface): Promise<number> {
  try {
    return await $.clock.now()
  } catch {
    return 0
  }
}

/** `$.process.run` as a Runner: a run that cannot start, or outlasts its bound, is `{ failed }`. */
const runnerOf =
  ($: EngineInterface): Runner =>
  async (argv, init) => {
    const started = await nowOf($)
    try {
      const r = await $.process.run(argv, init)
      return { exitCode: r.exitCode, stdout: r.stdout, stderr: r.stderr } satisfies Ran
    } catch {
      const elapsed = (await nowOf($)) - started
      return { failed: elapsed >= init.timeoutMs ? 'timed out' : 'not found' }
    }
  }

/** `$.fs.read` as a Reader: a missing or unreadable file is null. */
const readerOf =
  ($: EngineInterface): Reader =>
  async path => {
    try {
      const text = await $.fs.read(path)
      return typeof text === 'string' ? text : null
    } catch {
      return null
    }
  }

/**
 * The checkout this copy of the mod sits in: `git rev-parse --show-toplevel` from the plugin
 * folder, and whether this is Windows. Kept once found; a failure is tried again next time.
 */
async function hostOf($: EngineInterface): Promise<Host | null> {
  if (hostMemo !== null) return hostMemo
  const r = await runnerOf($)(['git', 'rev-parse', '--show-toplevel'], { cwd: $.plugin.root, timeoutMs: 10_000 })
  if ('failed' in r || r.exitCode !== 0 || r.stdout.trim() === '') return null
  let windows = false
  try {
    windows = (await $.env.get('OS')) === 'Windows_NT'
  } catch {
    // Not Windows, as far as anyone can tell.
  }
  hostMemo = { root: r.stdout.trim(), windows }
  return hostMemo
}

/** Toasts a gate that finished since the last one seen (C2); the first one seen only seeds. */
async function toastGate($: EngineInterface, doc: GateRead): Promise<void> {
  if (doc === null || doc === 'unreadable' || doc.state === 'running' || doc.finished === null) return
  const seen = await read($, gateSeen)
  if (seen !== null && seen.finished === doc.finished) return
  const finished = doc.finished
  const text = gateToast(doc)
  if (seen !== null && text !== null) $.ui.toast(text)
  await update($, gateSeen, () => ({ finished }))
}

/** Seeds `gateSeen` from the gate result there is now, so a gate that ended before the session never toasts. */
async function seedGate($: EngineInterface): Promise<void> {
  const host = await hostOf($)
  // No root, no seed: the first finished gate a tick sees then seeds instead of toasting.
  if (host === null) return
  const doc = parseGate(await readerOf($)(join(host.root, 'target', 'gate', 'last.json')))
  const finished = doc !== null && doc !== 'unreadable' && doc.state !== 'running' ? doc.finished : null
  await update($, gateSeen, prev => prev ?? { finished })
}

/** One period: gather the facts, compose the line, toast a finished gate, age the warning out. */
async function tick($: EngineInterface): Promise<void> {
  if (polling) return
  polling = true
  try {
    const now = await nowOf($)
    const host = await hostOf($)
    if (host === null) {
      ambientLine = 'htui-dev: no repo root (git rev-parse failed)'
    } else {
      let hrItem: string | undefined
      try {
        hrItem = await $.env.get('HR_ITEM')
      } catch {
        hrItem = undefined
      }
      const snap = await gather(host, hrItem, runnerOf($), readerOf($), scriptArgv(host, 'health'))
      ambientLine = composeAmbient(snap, now)
      try {
        await toastGate($, snap.gate)
      } catch {
        // The state store failed: no toast this time; the line still updates.
      }
    }
    if (warning !== null) {
      warning.at ??= now
      if (now - warning.at >= WARNING_MS) warning = null
    }
    show($)
  } catch {
    // The module unloaded mid-tick (its `$` is gone), or a state write failed: the next tick retries.
  } finally {
    polling = false
  }
}

/** Starts the status timer unless one runs, with one tick now. */
function arm($: EngineInterface): void {
  if (timer !== null) return
  timer = $.clock.every(PERIOD_MS, () => void tick($))
  void (async () => {
    try {
      await seedGate($)
    } catch {
      // Unseeded: the first finished gate seen then seeds instead of toasting.
    }
    await tick($)
  })()
}

/**
 * A reload drops the timer. Live (2.1.296) a reload raises `session.start` again, which re-arms
 * it; should one not, the first main-loop tool call after it asks `ambient` whether this session
 * ran the timer and re-arms it (or, with `status_line` now off, takes the ambient part off the
 * line). Never awaited: it adds nothing to a guard's path.
 */
function kick($: EngineInterface, agentId: string | undefined): void {
  // Only the main loop's calls: a subagent's `$` is not known to outlive its dispatch.
  if (agentId !== undefined || rearmChecked || timer !== null) return
  rearmChecked = true
  void (async () => {
    let fit: boolean
    try {
      fit = await read($, ambient)
    } catch {
      rearmChecked = false
      return
    }
    if (fit !== true) return
    if (statusLine) {
      arm($)
    } else {
      ambientLine = null
      show($, true)
    }
  })().catch(() => {})
}

// ---- M2: the commands ----

const NO_ROOT: Answer = { text: 'htui-dev: cannot find the repo root (git rev-parse --show-toplevel failed)', exitCode: 1 }

/** Runs a command's body against the checkout; never throws (a throw is the command's answer). */
async function answer($: EngineInterface, name: string, body: (h: Host) => Promise<Answer>): Promise<Answer> {
  try {
    const h = await hostOf($)
    return h === null ? NO_ROOT : await body(h)
  } catch (err) {
    return { text: `htui-dev: /${name} failed: ${err instanceof Error ? err.message : String(err)}`, exitCode: 1 }
  }
}

export const register: Register = (on, options) => {
  statusLine = (options as { status_line?: unknown }).status_line !== false
  timer = null
  polling = false
  rearmChecked = false
  warning = null
  ambientLine = null
  shown = undefined
  hostMemo = null

  on('session.start', async ($, e, next) => {
    // Every session gets the commands (they serve `claude -p "/validate"` too); each is registered
    // by a literal name, so `claude plugin validate` reads its hook as answering its own command.
    try {
      await $.command.register({
        name: 'gate',
        description: 'Run the htui gate detached (scripts/gate.sh); status: its last result',
        argumentHint: '[step|status|force]',
      })
      await $.command.register({ name: 'next', description: 'Rank the open HANDOFF items (next-item script)' })
      await $.command.register({ name: 'validate', description: 'Check HANDOFF.md and DECISIONS.md (workflow validator)' })
    } catch {
      warn($, 'htui-dev: /gate, /next or /validate not registered')
    }
    // The timer runs where a person watches the line, and never in a session htui drives.
    try {
      const fit = e.isInteractive === true && (await $.env.get('HTUI_SESSION')) === undefined
      await update($, ambient, () => fit)
      if (fit && statusLine) arm($)
    } catch {
      // No timer; the commands still work.
    }
    return next(e)
  })

  on('command.run', { command: 'gate' }, async ($, e) =>
    answer($, 'gate', async h => {
      const run = runnerOf($)
      const head = await run(['git', 'rev-parse', 'HEAD'], { cwd: h.root, timeoutMs: 10_000 })
      const sha = 'failed' in head || head.exitCode !== 0 ? null : head.stdout.trim()
      return gateCommand(e.args, h, run, readerOf($), await nowOf($), sha)
    }),
  )
  on('command.run', { command: 'next' }, async $ => answer($, 'next', h => scriptCommand('next', h, runnerOf($))))
  on('command.run', { command: 'validate' }, async $ => answer($, 'validate', h => scriptCommand('validate', h, runnerOf($))))

  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    kick($, e.agentId)
    let cmds: Words[] | null = null
    try {
      cmds = simpleCommands(e.command)
    } catch {
      warn($, 'htui-dev: shell parser failed, call allowed')
    }
    const tagged = await isTagged($, e.agentId, 'htui-dev: read-only guard failed, call allowed')
    if (cmds === null) {
      // A line the parser cannot follow is allowed, with no status: it is no failure of the guard.
      // A read-only agent's still meets a coarse check for git writes.
      const reason = tagged
        ? judge($, 'read-only', 'call', () => {
            const verb = coarseGitWrite(e.command)
            return verb === null ? null : readOnlyGit(verb, e.agentId!)
          })
        : null
      return reason !== null ? { deny: reason } : next(e)
    }
    // A `cd crates/<member>` earlier in the line makes a bare `cargo test` select that member.
    let cwd: Cwd = []
    for (const words of cmds) {
      const reason =
        (tagged
          ? judge($, 'read-only', 'call', () => {
              const verb = gitWrite(words)
              return verb === null ? null : readOnlyGit(verb, e.agentId!)
            })
          : null) ??
        judge($, 'no-verify', 'call', () => noVerify(words)) ??
        judge($, 'cargo test', 'call', () => cargoTest(words, cwd))
      if (reason !== null) return { deny: reason }
      try {
        cwd = movedTo(words, cwd)
      } catch {
        cwd = []
      }
    }
    return next(e)
  }).catch(($, e, next) => {
    // A backstop (overrun budget, wrong answer shape, re-entry): it replays a settled `next`, and
    // otherwise lets the call through with a warning. It never refuses.
    if (next.called) return next(e)
    warn($, 'htui-dev: tool.call guard failed, call allowed')
    return next(e)
  })

  // The tools that write a file. Of Gortex's MCP tools (Q3): `edit_file`, `write_file`,
  // `edit_symbol`, `rename_symbol` and `batch_edit` by their schemas; `edit` and `refactor`, the
  // compact names, read the same way. Serena's write tools are configured machine-wide, so they
  // reach this repo too; its optional JetBrains ones are `jet_brains_*` in Serena 2.0.0 (the
  // shorter `jetbrains_*` is matched as well). `execute_shell_command` is left out: Serena's
  // claude-code context excludes it, and it is no file write. Either server may also come from a plugin (`mcp__plugin_<name>_<server>__`).
  // The patterns are written in the matcher, so that `claude plugin validate` lists them.
  on(
    'tool.call',
    {
      tool: [
        'Edit',
        'Write',
        'NotebookEdit',
        /^mcp__(plugin_[a-z0-9_-]+_)?gortex__(edit|edit_file|edit_symbol|batch_edit|rename_symbol|refactor|write_file)$/,
        /^mcp__(plugin_[a-z0-9_-]+_)?serena__(create_text_file|replace_content|replace_in_files|replace_symbol_body|insert_after_symbol|insert_before_symbol|rename_symbol|safe_delete_symbol|replace_lines|insert_at_line|delete_lines|jet_?brains_(?:rename|move|safe_delete|inline_symbol))$/,
      ],
    },
    async ($, e, next) => {
      kick($, e.agentId)
      const input = e as unknown as Readonly<Record<string, unknown>>
      if (writesNothing(e.tool, input)) return next(e)
      // G4 first: a read-only agent writes nothing, whatever the text.
      if (await isTagged($, e.agentId, 'htui-dev: read-only guard failed, call allowed')) {
        return { deny: readOnlyEdit(e.tool, e.agentId!) }
      }
      const reason = judge($, 'forbidden literal', 'call', () => forbiddenLiteral(e.tool, input))
      return reason !== null ? { deny: reason } : next(e)
    },
  ).catch(($, e, next) => {
    // A backstop (overrun budget, wrong answer shape, re-entry): it replays a settled `next`, and
    // otherwise lets the call through with a warning. It never refuses.
    if (next.called) return next(e)
    warn($, 'htui-dev: tool.call guard failed, call allowed')
    return next(e)
  })

  // G4 for workflows: a workflow agent's spawn names no parent, only the Workflow call that
  // started its run (its `tool_use_id`). A read-only agent's Workflow and Agent calls are noted
  // before they run, so the spawns they cause can be tagged. Never refuses.
  on('tool.call', { tool: ['Workflow', 'Agent'] }, async ($, e, next) => {
    kick($, e.agentId)
    if (await isTagged($, e.agentId, 'htui-dev: read-only guard failed, call allowed')) {
      const id = e.tool_use_id
      await store($, () => update($, readOnlyCalls, adding(id)), `htui-dev: read-only tag failed, call ${id} unrecorded`)
    }
    return next(e)
  }).catch(($, e, next) => {
    if (next.called) return next(e)
    warn($, 'htui-dev: tool.call guard failed, call allowed')
    return next(e)
  })

  on('agent.spawn', async ($, e, next) => {
    // Every refusal comes before `next`: a deny after it fails the hook (AgentSpawnResult).
    const fable = judge($, 'Fable', 'spawn', () => fableModel(e.model))
    if (fable !== null) return { deny: fable }
    let tag = false
    try {
      tag = declaresReadOnly(e)
    } catch {
      warn($, 'htui-dev: read-only guard failed, spawn allowed untagged')
    }
    if (!tag) tag = await isTagged($, e.parentAgentId, 'htui-dev: read-only guard failed, spawn allowed untagged')
    // A main-loop spawn that is no workflow agent reads no state.
    if (!tag && (e.workflow !== undefined || e.parentAgentId !== undefined)) {
      tag = await isReadOnlyCall($, e.tool_use_id, 'htui-dev: read-only guard failed, spawn allowed untagged')
    }
    const got = await next(e)
    if (got.deny !== undefined) return got
    if (tag && got.agentId !== undefined) {
      const id = got.agentId
      await store($, () => update($, readOnly, adding(id)), `htui-dev: read-only tag failed, agent ${id} untagged`)
    }
    // G5's gap: a model left unset resolves from the agent definition or the parent, after the
    // hook could refuse. Too late to deny (it fails the hook); say so instead.
    if (typeof got.model === 'string' && isFable(got.model)) {
      warn($, `htui-dev: subagent ${got.agentId ?? '(no id)'} resolved to Fable (${got.model}) — inherited or pinned in its agent definition`)
    }
    return got
  }).catch(spawnBackstop)
}

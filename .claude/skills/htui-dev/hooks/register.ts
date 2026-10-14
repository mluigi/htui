// htui-dev: guards for developing htui. Each refuses, with the fix in its reason, a kind of
// call that has cost this repo real work (docs/htui-dev.md):
//   G1  cargo test that builds feature-gated integration tests without their feature (0 tests, green)
//   G2  cargo test with the other crate's test feature name (testkit vs test-support)
//   G3  the literal extensibility.rs forbids, written into crates/**/*.{rs,json} by an edit tool
//   G4  file writes, and git commands that change the tree, index or history, from an agent started read-only
//   G5  a subagent asked to run on Fable
//   G6  git commit --no-verify (or -n), which skips the repo's commit hooks
// Every guard is a `{ deny }`, never a rewrite, and judges before `next`. A guard that throws
// lets the call through and says so on the status line (ANA-29 §6.3, "fail open"): the guards
// are a safety net, not a sandbox. The mod runs no process, touches no file and reaches no
// network; its one value is the read-only agents' ids in `$.state`, which a reload keeps.
import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register } from 'claude-code'
import type { HtuiDevReadOnly } from '../types'
import {
  cargoTest,
  declaresReadOnly,
  fableModel,
  forbiddenLiteral,
  gitWrite,
  isFable,
  noVerify,
  readOnlyEdit,
  readOnlyGit,
  writesNothing,
} from './guards'
import { simpleCommands, type Words } from './shell'

const readOnly = atom({ plugin: 'htui-dev', key: 'readOnly' } as const, [] as HtuiDevReadOnly)

/** Puts `text` on the plugin's status line; never throws (a `$` call in a re-entry rejects). */
function warn($: EngineInterface, text: string): void {
  try {
    $.ui.status(text)
  } catch {
    // Nothing is left to tell.
  }
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
 * Whether `agentId` was tagged read-only. The main loop (no `agentId`) never reads the state. A
 * read that fails, or a value that is no list, warns with `failed` and answers false.
 */
async function isTagged($: EngineInterface, agentId: string | undefined, failed: string): Promise<boolean> {
  if (agentId === undefined) return false
  try {
    return (await read($, readOnly)).includes(agentId)
  } catch {
    warn($, failed)
    return false
  }
}

export const register: Register = on => {
  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    let cmds: Words[] | null = null
    try {
      cmds = simpleCommands(e.command)
    } catch {
      warn($, 'htui-dev: shell parser failed, call allowed')
    }
    // A line the parser cannot follow is allowed, with no status: it is no failure of the guard.
    if (cmds === null) return next(e)
    const tagged = await isTagged($, e.agentId, 'htui-dev: read-only guard failed, call allowed')
    for (const words of cmds) {
      const reason =
        (tagged
          ? judge($, 'read-only', 'call', () => {
              const verb = gitWrite(words)
              return verb === null ? null : readOnlyGit(verb, e.agentId!)
            })
          : null) ??
        judge($, 'no-verify', 'call', () => noVerify(words)) ??
        judge($, 'cargo test', 'call', () => cargoTest(words))
      if (reason !== null) return { deny: reason }
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
  // reach this repo too. Either server may also come from a plugin (`mcp__plugin_<name>_<server>__`).
  // The patterns are written in the matcher, so that `claude plugin validate` lists them.
  on(
    'tool.call',
    {
      tool: [
        'Edit',
        'Write',
        'NotebookEdit',
        /^mcp__(plugin_[a-z0-9_-]+_)?gortex__(edit|edit_file|edit_symbol|batch_edit|rename_symbol|refactor|write_file)$/,
        /^mcp__(plugin_[a-z0-9_-]+_)?serena__(create_text_file|replace_content|replace_in_files|replace_symbol_body|insert_after_symbol|insert_before_symbol|rename_symbol|safe_delete_symbol|replace_lines|insert_at_line|delete_lines)$/,
      ],
    },
    async ($, e, next) => {
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
    const got = await next(e)
    if (got.deny !== undefined) return got
    if (tag && got.agentId !== undefined) {
      const id = got.agentId
      try {
        await update($, readOnly, ids => ((ids ?? []).includes(id) ? ids : [...(ids ?? []), id]))
      } catch {
        warn($, `htui-dev: read-only tag failed, agent ${id} untagged`)
      }
    }
    // G5's gap: a model left unset resolves from the agent definition or the parent, after the
    // hook could refuse. Too late to deny (it fails the hook); say so instead.
    if (typeof got.model === 'string' && isFable(got.model)) {
      warn($, `htui-dev: subagent ${got.agentId ?? '(no id)'} resolved to Fable (${got.model}) — inherited or pinned in its agent definition`)
    }
    return got
  }).catch(($, e, next) => {
    // The same backstop: a spawn is never refused here, and a settled `next` is replayed.
    if (next.called) return next(e)
    warn($, 'htui-dev: agent.spawn guard failed, spawn allowed')
    return next(e)
  })
}

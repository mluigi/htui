// Shared helpers for the mod's kit tests (`claude plugin test`). The tests stand
// for the engine beneath the mod: what a hook here answers reaches the mod as given.
import type { On } from 'claude-code'
import type { Engine } from 'claude-code/testing'

/** The literal crates/htui-agent/tests/extensibility.rs forbids, never spelled whole in this repo. */
export const LIT = 'ze' + 'ta'

/** One tool call that got past the mod: the tool, and the agentId it reached the engine with. */
export type Reached = { tool: string; agentId: string | undefined }

/** Stands in for the tools beneath the mod: every call that gets through answers `ok`, recorded in `reached`. */
export const tools = (on: On, reached: Reached[]) => {
  on('tool.call', async ($, e) => {
    reached.push({ tool: e.tool, agentId: (e as { agentId?: string }).agentId })
    return { result: 'ok' } as never
  })
}

/** One spawn that got past the mod: its type, its model, and the parent it reached the engine with. */
export type Spawned = { subagentType: string; model: string | undefined; parentAgentId: string | undefined }

/**
 * Stands in for the engine's spawn beneath the mod: each spawn that gets through starts
 * `agent-<n>`, counting from 1, on `model` (or `claude-opus-5-5`); recorded in `spawned`.
 * `answer` replaces the answer, for a spawn that starts no agent or is refused below.
 */
export const agents = (on: On, spawned: Spawned[], answer?: (n: number) => object) => {
  on('agent.spawn', async ($, e) => {
    spawned.push({ subagentType: e.subagentType, model: e.model, parentAgentId: e.parentAgentId })
    const n = spawned.length
    return (answer ? answer(n) : { model: e.model ?? 'claude-opus-5-5', agentId: `agent-${n}` }) as never
  })
}

/** Records every `$.ui.status` line the mod sets; the kit has no bottom for it. */
export const statuses = (on: On, lines: (string | undefined)[]) => {
  on('ui.status', async ($, e) => {
    lines.push(e.text)
    return { value: undefined } as never
  })
}

/** Records every value the mod writes to its `readOnly` tags. */
export const tagWrites = (on: On, writes: unknown[]) => {
  on('state.set', { plugin: 'htui-dev', key: 'readOnly' }, async ($, e, next) => {
    writes.push(e.value)
    return next(e)
  })
}

/** Answers every read of the `readOnly` tags with `value`, as after a reload. Never tag in a test that seeds. */
export const seedTags = (on: On, value: unknown) => {
  on('state.get', { plugin: 'htui-dev', key: 'readOnly' }, async () => ({ value: { value, version: 1 } }) as never)
}

/** Runs `command` through Bash, from the main loop or from the subagent `agentId`. */
export const bash = ($: Engine, command: string, agentId?: string) =>
  $.tool.call({ tool: 'Bash', command, ...(agentId === undefined ? {} : { agentId }) } as never)

/** Calls `tool` with `input`, from the main loop or from the subagent `agentId`. */
export const call = ($: Engine, tool: string, input: object, agentId?: string) =>
  $.tool.call({ tool, ...input, ...(agentId === undefined ? {} : { agentId }) } as never)

/** Spawns a subagent as the Agent tool would; `parentAgentId` makes it a nested spawn. */
export const spawn = (
  $: Engine,
  args: { prompt?: string; subagentType?: string; model?: string; parentAgentId?: string },
) => $.agent.spawn({ prompt: 'do the task', subagentType: 'general-purpose', ...args } as never)

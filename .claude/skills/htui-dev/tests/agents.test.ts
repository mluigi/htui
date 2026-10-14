import { test, expect } from 'claude-code/testing'
import { agents, bash, spawn, tools, type Reached, type Spawned } from './fixtures'

// A probe of the kit itself: G4's tests below hand the mod an agentId and a parentAgentId.
// Should the kit drop either, those cases are tested through the pure functions instead.
test("a subagent's call reaches the guards with its agentId", async ($, on) => {
  const reached: Reached[] = []
  tools(on, reached)
  await bash($, 'true', 'agent-1')
  expect(reached).toEqual([{ tool: 'Bash', agentId: 'agent-1' }])
})

test("a nested spawn reaches the guards with its parentAgentId", async ($, on) => {
  const spawned: Spawned[] = []
  agents(on, spawned)
  await spawn($, { parentAgentId: 'agent-1' })
  expect(spawned).toEqual([{ subagentType: 'general-purpose', model: undefined, parentAgentId: 'agent-1' }])
})

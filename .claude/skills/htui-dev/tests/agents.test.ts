import { test, expect, mock } from 'claude-code/testing'
import { agents, bash, call, LIT, seedTags, spawn, statuses, tagWrites, tools, type Reached, type Spawned } from './fixtures'

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

test('each read-only marker tags the agent it starts', async ($, on) => {
  const reached: Reached[] = []
  const spawned: Spawned[] = []
  const writes: unknown[] = []
  tools(on, reached)
  agents(on, spawned)
  tagWrites(on, writes)
  const markers = [
    { prompt: 'READ-ONLY verifier. Check the diff.' },
    { prompt: 'Review it. STRICTLY no edits, no commits.' },
    { prompt: 'review', subagentType: 'rust-reviewer' },
    { prompt: 'design', subagentType: 'code-architect' },
  ]
  for (const args of markers) await spawn($, args)
  expect(writes).toEqual([['agent-1'], ['agent-1', 'agent-2'], ['agent-1', 'agent-2', 'agent-3'], ['agent-1', 'agent-2', 'agent-3', 'agent-4']])
  for (const id of ['agent-1', 'agent-2', 'agent-3', 'agent-4']) {
    const r = (await call($, 'Edit', { file_path: 'src/a.rs', old_string: 'a', new_string: 'b' }, id)) as { deny?: string }
    expect(r.deny, id).toBe(`htui-dev: agent ${id} was started read-only and may not change files (Edit) — report the change in your answer instead`)
  }
  expect(reached).toEqual([])
})

test('an agent spawned by a read-only agent is read-only too', async ($, on) => {
  const writes: unknown[] = []
  const spawned: Spawned[] = []
  agents(on, spawned)
  tagWrites(on, writes)
  await spawn($, { prompt: 'READ-ONLY: look around' })
  await spawn($, { prompt: 'write the fix', parentAgentId: 'agent-1' })
  await spawn($, { prompt: 'write the fix', parentAgentId: 'agent-9' })
  expect(writes).toEqual([['agent-1'], ['agent-1', 'agent-2']])
})

test('a lowercase read-only in the prompt tags nothing', async ($, on) => {
  const writes: unknown[] = []
  const reached: Reached[] = []
  tools(on, reached)
  agents(on, [])
  tagWrites(on, writes)
  await spawn($, { prompt: 'Fix the bug; leave the read-only host tree alone.' })
  await spawn($, { prompt: 'Strictly, no edit here is too small.', subagentType: 'Explore' })
  expect(writes).toEqual([])
  await call($, 'Edit', { file_path: 'src/a.rs', old_string: 'a', new_string: 'b' }, 'agent-1')
  expect(reached).toEqual([{ tool: 'Edit', agentId: 'agent-1' }])
})

test('a spawn that starts no agent tags nothing', async ($, on) => {
  const writes: unknown[] = []
  agents(on, [], n => (n === 1 ? { model: 'claude-opus-5-5' } : { deny: 'refused below' }))
  tagWrites(on, writes)
  expect(await spawn($, { prompt: 'READ-ONLY' })).toEqual({ model: 'claude-opus-5-5' })
  expect(await spawn($, { prompt: 'READ-ONLY' })).toEqual({ deny: 'refused below' })
  expect(writes).toEqual([])
})

test('a read-only agent may not write files', async ($, on) => {
  const reached: Reached[] = []
  tools(on, reached)
  agents(on, [])
  await spawn($, { prompt: 'READ-ONLY' })
  const writes: [string, object][] = [
    ['Edit', { file_path: 'src/a.rs', old_string: 'a', new_string: 'b' }],
    ['Write', { file_path: 'src/a.rs', content: 'b' }],
    ['NotebookEdit', { notebook_path: 'a.ipynb', new_source: 'b' }],
    ['mcp__gortex__edit_file', { path: 'src/a.rs', old_string: 'a', new_string: 'b' }],
    ['mcp__gortex__write_file', { path: 'src/a.rs', content: 'b' }],
    ['mcp__gortex__batch_edit', { edits: [] }],
    ['mcp__plugin_gortex_gortex__edit_symbol', { id: 'htui/src/a.rs::f', old_source: 'a', new_source: 'b' }],
    ['mcp__serena__replace_symbol_body', { name_path: 'f', relative_path: 'src/a.rs', body: 'b' }],
    ['mcp__serena__replace_in_files', { needle: 'a', repl: 'b', mode: 'literal' }],
    ['mcp__serena__safe_delete_symbol', { name_path_pattern: 'f', relative_path: 'src/a.rs' }],
    ['mcp__serena__delete_lines', { relative_path: 'src/a.rs', start_line: 1, end_line: 2 }],
    ['mcp__plugin_serena_serena__rename_symbol', { name_path: 'f', relative_path: 'src/a.rs', new_name: 'g' }],
  ]
  for (const [tool, input] of writes) {
    const r = (await call($, tool, input, 'agent-1')) as { deny?: string }
    expect(r.deny, tool).toBe(`htui-dev: agent agent-1 was started read-only and may not change files (${tool}) — report the change in your answer instead`)
  }
  expect(reached).toEqual([])
})

test('a read-only agent may not change the tree, the index or history through git', async ($, on) => {
  const reached: Reached[] = []
  tools(on, reached)
  agents(on, [])
  await spawn($, { prompt: 'READ-ONLY' })
  const commands: [string, string][] = [
    ['git commit -m x', 'git commit'],
    ['git stash', 'git stash'],
    ['git reset --hard', 'git reset'],
    ['git checkout -- src/a.rs', 'git checkout <paths>'],
    ['git checkout .', 'git checkout <paths>'],
    ['git restore .', 'git restore'],
    ['git clean -fdx', 'git clean'],
    ['git add -A', 'git add'],
    ['git branch -D topic', 'git branch'],
    ['cd crates && git commit -am x', 'git commit'],
    ['git -C /r commit --no-verify -m x', 'git commit'],
  ]
  for (const [command, verb] of commands) {
    const r = (await bash($, command, 'agent-1')) as { deny?: string }
    expect(r.deny, command).toBe(`htui-dev: agent agent-1 was started read-only and may not run ${verb} — report what to change instead`)
  }
  expect(reached).toEqual([])
})

test('a read-only agent may read, check and inspect', async ($, on) => {
  const reached: Reached[] = []
  tools(on, reached)
  agents(on, [])
  await spawn($, { prompt: 'READ-ONLY' })
  await call($, 'Read', { file_path: 'src/a.rs' }, 'agent-1')
  await bash($, 'cargo check -p htui', 'agent-1')
  await bash($, 'git status && git diff && git stash list', 'agent-1')
  await bash($, 'git log -n 3 && git show HEAD && git branch -a && git fetch && git checkout main', 'agent-1')
  await call($, 'mcp__gortex__search', { query: 'x' }, 'agent-1')
  await call($, 'mcp__serena__replace_in_files', { needle: 'a', repl: 'b', mode: 'literal', dry_run: true }, 'agent-1')
  expect(reached.map(r => r.tool)).toEqual(['Read', 'Bash', 'Bash', 'Bash', 'mcp__gortex__search', 'mcp__serena__replace_in_files'])
})

test('a read-only agent still meets the other guards', async ($, on) => {
  const reached: Reached[] = []
  tools(on, reached)
  agents(on, [])
  await spawn($, { prompt: 'READ-ONLY' })
  const r = (await bash($, 'cargo test -p htui', 'agent-1')) as { deny?: string }
  expect(r.deny).toMatch(/^htui-dev: `cargo test -p htui` builds htui's integration tests/)
})

test('an untagged agent and the main loop may write and commit', async ($, on) => {
  const reached: Reached[] = []
  tools(on, reached)
  agents(on, [])
  await spawn($, { prompt: 'implement it' })
  await call($, 'Edit', { file_path: 'src/a.rs', old_string: 'a', new_string: 'b' }, 'agent-1')
  await bash($, 'git commit -m x', 'agent-1')
  await call($, 'Edit', { file_path: 'src/a.rs', old_string: 'a', new_string: 'b' })
  await bash($, 'git commit -m x')
  expect(reached).toEqual([
    { tool: 'Edit', agentId: 'agent-1' },
    { tool: 'Bash', agentId: 'agent-1' },
    { tool: 'Edit', agentId: undefined },
    { tool: 'Bash', agentId: undefined },
  ])
})

test('a tag outlives a reload: the state holds it, not the module', async ($, on) => {
  const reached: Reached[] = []
  tools(on, reached)
  seedTags(on, ['agent-1'])
  const r = (await call($, 'Write', { file_path: 'src/a.rs', content: 'b' }, 'agent-1')) as { deny?: string }
  expect(r.deny).toMatch(/^htui-dev: agent agent-1 was started read-only/)
  await call($, 'Write', { file_path: 'src/a.rs', content: 'b' }, 'agent-2')
  expect(reached).toEqual([{ tool: 'Write', agentId: 'agent-2' }])
})

test('a Fable subagent is refused before it starts', async ($, on) => {
  const spawned: Spawned[] = []
  const writes: unknown[] = []
  agents(on, spawned)
  tagWrites(on, writes)
  for (const model of ['fable', 'claude-fable-5', 'FABLE']) {
    expect(await spawn($, { prompt: 'READ-ONLY', model }), model).toEqual({
      deny: `htui-dev: subagents never run on Fable in this repo (model "${model}") — leave model unset so the agent inherits the session's model`,
    })
  }
  expect(spawned).toEqual([])
  expect(writes).toEqual([])
})

test('any other model, or none, starts', async ($, on) => {
  const spawned: Spawned[] = []
  agents(on, spawned)
  await spawn($, { model: 'opus' })
  await spawn($, {})
  expect(spawned.map(s => s.model)).toEqual(['opus', undefined])
})

test('a subagent that resolves to Fable on its own is let through with a status', async ($, on) => {
  const clock = mock.clock(on)
  const lines: (string | undefined)[] = []
  statuses(on, lines)
  agents(on, [], n => ({ model: 'claude-fable-5', agentId: `agent-${n}` }))
  expect(await spawn($, {})).toEqual({ model: 'claude-fable-5', agentId: 'agent-1' })
  await clock.settle()
  expect(lines).toEqual([
    'htui-dev: subagent agent-1 resolved to Fable (claude-fable-5) — inherited or pinned in its agent definition',
  ])
})

// Fail open (ANA-29 §6.3): a guard that cannot judge lets the call through and says so.

test('an unreadable tag store lets a write through, with a status', async ($, on) => {
  const clock = mock.clock(on)
  const lines: (string | undefined)[] = []
  const reached: Reached[] = []
  statuses(on, lines)
  tools(on, reached)
  seedTags(on, 42)
  await call($, 'Edit', { file_path: 'src/a.rs', old_string: 'a', new_string: 'b' }, 'agent-1')
  await bash($, 'git commit -m x', 'agent-1')
  await clock.settle()
  expect(reached.map(r => r.tool)).toEqual(['Edit', 'Bash'])
  expect(lines).toEqual([
    'htui-dev: read-only guard failed, call allowed',
    'htui-dev: read-only guard failed, call allowed',
  ])
})

test('an unreadable tag store lets a nested spawn start untagged, with a status', async ($, on) => {
  const clock = mock.clock(on)
  const lines: (string | undefined)[] = []
  const spawned: Spawned[] = []
  statuses(on, lines)
  agents(on, spawned)
  seedTags(on, 42)
  expect(await spawn($, { parentAgentId: 'agent-9' })).toEqual({ model: 'claude-opus-5-5', agentId: 'agent-1' })
  await clock.settle()
  expect(spawned).toHaveLength(1)
  expect(lines).toEqual(['htui-dev: read-only guard failed, spawn allowed untagged'])
})

test('a tag that cannot be written leaves the agent started, with a status', async ($, on) => {
  const clock = mock.clock(on)
  const lines: (string | undefined)[] = []
  statuses(on, lines)
  agents(on, [])
  on('state.set', { plugin: 'htui-dev', key: 'readOnly' }, async () => ({ deny: 'store refused' }) as never)
  expect(await spawn($, { prompt: 'READ-ONLY' })).toEqual({ model: 'claude-opus-5-5', agentId: 'agent-1' })
  await clock.settle()
  expect(lines).toEqual(['htui-dev: read-only tag failed, agent agent-1 untagged'])
})

test('a Bash call the parser throws on goes through, with a status', async ($, on) => {
  const clock = mock.clock(on)
  const lines: (string | undefined)[] = []
  const reached: Reached[] = []
  statuses(on, lines)
  tools(on, reached)
  await $.tool.call({ tool: 'Bash', command: undefined } as never)
  await clock.settle()
  expect(reached.map(r => r.tool)).toEqual(['Bash'])
  expect(lines).toEqual(['htui-dev: shell parser failed, call allowed'])
})

test('refusing and allowing set no status line', async ($, on) => {
  const clock = mock.clock(on)
  const lines: (string | undefined)[] = []
  statuses(on, lines)
  tools(on, [])
  agents(on, [])
  await spawn($, { prompt: 'READ-ONLY' })
  await spawn($, { model: 'fable' })
  await spawn($, {})
  await call($, 'Edit', { file_path: 'src/a.rs', old_string: 'a', new_string: 'b' }, 'agent-1')
  await call($, 'Edit', { file_path: 'src/a.rs', old_string: 'a', new_string: 'b' }, 'agent-2')
  await call($, 'Write', { file_path: 'crates/htui/src/a.rs', content: LIT })
  await bash($, 'git commit -n -m x && cargo test -p htui')
  await bash($, 'git commit -m x', 'agent-1')
  await bash($, 'cargo test -p htui --features testkit', 'agent-2')
  await clock.settle()
  expect(lines).toEqual([])
})

import { test, expect } from 'claude-code/testing'
import type { On } from 'claude-code'
import type { Engine } from 'claude-code/testing'
import { call, LIT, tools, type Reached } from './fixtures'

/** One call: the tool and its input. */
type Call = [tool: string, input: object]

/** Makes each call from the main loop and answers the deny reason, or undefined when it went through. */
const verdicts = async ($: Engine, on: On, calls: readonly Call[]) => {
  const reached: Reached[] = []
  tools(on, reached)
  const out: (string | undefined)[] = []
  for (const [tool, input] of calls) {
    const before = reached.length
    const r = (await call($, tool, input)) as { deny?: string }
    expect(reached.length, `${tool} ${JSON.stringify(input)}`).toBe(r.deny === undefined ? before + 1 : before)
    out.push(r.deny)
  }
  return out
}

/** A text that names the literal, in an identifier as code would. */
const NAMED = `fn ${LIT}_row() {}`

test('the literal written into a crate source is refused', async ($, on) => {
  const calls: Call[] = [
    ['Edit', { file_path: '/r/crates/htui-core/src/a.rs', old_string: 'x', new_string: NAMED }],
    ['Write', { file_path: 'crates/htui/seeds/a.json', content: `{"agent":"${LIT}"}` }],
    ['Edit', { file_path: 'C:\\r\\crates\\htui\\src\\a.rs', old_string: 'x', new_string: NAMED }],
    ['Edit', { file_path: '/r/.claude/worktrees/x/crates/htui/tests/a.rs', old_string: 'x', new_string: NAMED }],
    ['Write', { file_path: 'crates/htui/build.rs', content: NAMED }],
    // Only the agent's own copy is exempt.
    ['Write', { file_path: 'crates/htui-core/tests/extensibility.rs', content: NAMED }],
  ]
  const got = await verdicts($, on, calls)
  calls.forEach((c, i) => expect(got[i], JSON.stringify(c)).toMatch(/^htui-dev: /))
})

test('each Gortex write tool is checked', async ($, on) => {
  const calls: Call[] = [
    ['mcp__gortex__edit_file', { path: 'crates/htui/src/a.rs', old_string: 'x', new_string: NAMED }],
    ['mcp__gortex__write_file', { path: '/r/crates/htui/src/a.rs', content: NAMED }],
    ['mcp__gortex__edit_symbol', { id: 'htui/crates/htui-core/src/a.rs::row', old_source: 'x', new_source: NAMED }],
    ['mcp__gortex__rename_symbol', { id: 'htui/crates/htui-core/src/a.rs::row', new_name: `${LIT}_row` }],
    ['mcp__gortex__edit', { target: { file: 'crates/htui/src/a.rs' }, old_string: 'x', new_string: NAMED }],
    ['mcp__gortex__refactor', { target: { symbol: 'htui/crates/htui/src/a.rs::row' }, new_name: `${LIT}_row` }],
    [
      'mcp__gortex__batch_edit',
      {
        edits: [
          { id: 'htui/crates/htui/src/a.rs::f', old_source: 'a', new_source: 'b' },
          { op: 'edit_file', path: 'crates/htui-store/src/b.rs', old_string: 'x', new_string: NAMED },
        ],
      },
    ],
  ]
  const got = await verdicts($, on, calls)
  calls.forEach((c, i) => expect(got[i], c[0]).toMatch(/^htui-dev: /))
})

test('a batch refusal names the edit that offends', async ($, on) => {
  const [got] = await verdicts($, on, [
    [
      'mcp__gortex__batch_edit',
      {
        edits: [
          { op: 'edit_file', path: 'docs/a.md', old_string: 'x', new_string: NAMED },
          { op: 'edit_file', path: 'crates/htui-store/src/b.rs', old_string: 'x', new_string: NAMED },
        ],
      },
    ],
  ])
  expect(got).toBe(
    `htui-dev: crates/htui-store/src/b.rs would contain "${LIT}", which crates/htui-agent/tests/extensibility.rs forbids in crates/**/*.{rs,json} — use another name; only extensibility.rs may spell it`,
  )
})

test('writes the scan does not read are allowed', async ($, on) => {
  const calls: Call[] = [
    ['Write', { file_path: 'crates/htui/target/debug/build/x/out/a.rs', content: NAMED }],
    ['Write', { file_path: '/r/crates/htui/target/a.json', content: NAMED }],
    ['Edit', { file_path: 'crates/htui-agent/tests/extensibility.rs', old_string: 'x', new_string: NAMED }],
    ['Edit', { file_path: '/r/crates/htui-agent/tests/extensibility.rs', old_string: 'x', new_string: NAMED }],
    ['Write', { file_path: 'crates/htui/claude-plugin/htui/hooks/a.ts', content: NAMED }],
    ['Write', { file_path: 'crates/htui/README.md', content: NAMED }],
    ['Write', { file_path: 'crates/htui/Cargo.toml', content: NAMED }],
    ['Write', { file_path: 'docs/a.rs', content: NAMED }],
    ['Write', { file_path: '.claude/skills/x.rs', content: NAMED }],
    ['NotebookEdit', { notebook_path: 'crates/htui/a.ipynb', new_source: NAMED }],
  ]
  const got = await verdicts($, on, calls)
  calls.forEach((c, i) => expect(got[i], JSON.stringify(c)).toBeUndefined())
})

test('removing the literal is allowed: only the new text is read', async ($, on) => {
  const got = await verdicts($, on, [
    ['Edit', { file_path: 'crates/htui/src/a.rs', old_string: NAMED, new_string: 'fn row() {}' }],
    ['mcp__gortex__edit_symbol', { id: 'htui/crates/htui/src/a.rs::f', old_source: NAMED, new_source: 'x' }],
  ])
  expect(got).toEqual([undefined, undefined])
})

test('the scan is case-sensitive, and so is the guard', async ($, on) => {
  const upper = LIT[0]!.toUpperCase() + LIT.slice(1)
  const got = await verdicts($, on, [
    ['Write', { file_path: 'crates/htui/src/a.rs', content: `// ${upper} and ${LIT.toUpperCase()}` }],
  ])
  expect(got).toEqual([undefined])
})

test('Gortex read tools are not matched', async ($, on) => {
  const got = await verdicts($, on, [
    ['mcp__gortex__read_file', { path: 'crates/htui/src/a.rs', content: NAMED }],
    ['mcp__gortex__search_text', { query: LIT, path: 'crates/htui/src/a.rs' }],
  ])
  expect(got).toEqual([undefined, undefined])
})

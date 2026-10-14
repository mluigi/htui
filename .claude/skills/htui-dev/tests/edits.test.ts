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

// Serena's write tools, with the input fields of this session's Serena tool list (create_text_file,
// replace_lines and insert_at_line were not listed; their fields are Serena's documented ones).
test('each Serena write tool is checked, also under a plugin prefix', async ($, on) => {
  const calls: Call[] = [
    ['mcp__serena__create_text_file', { relative_path: 'crates/htui/src/a.rs', content: NAMED }],
    ['mcp__serena__replace_content', { relative_path: 'crates/htui/src/a.rs', needle: 'x', repl: NAMED, mode: 'literal' }],
    ['mcp__serena__replace_symbol_body', { name_path: 'row', relative_path: 'crates/htui-core/src/a.rs', body: NAMED }],
    ['mcp__serena__insert_after_symbol', { name_path: 'row', relative_path: 'crates/htui-core/src/a.rs', body: NAMED }],
    ['mcp__serena__insert_before_symbol', { name_path: 'row', relative_path: 'crates/htui-core/src/a.rs', body: NAMED }],
    ['mcp__serena__rename_symbol', { name_path: 'row', relative_path: 'crates/htui-core/src/a.rs', new_name: `${LIT}_row` }],
    ['mcp__serena__replace_lines', { relative_path: 'crates/htui/seeds/a.json', start_line: 1, end_line: 2, content: NAMED }],
    ['mcp__serena__insert_at_line', { relative_path: 'crates/htui/src/a.rs', line: 3, content: NAMED }],
    ['mcp__plugin_serena_serena__replace_content', { relative_path: 'crates/htui/src/a.rs', needle: 'x', repl: NAMED, mode: 'regex' }],
    ['mcp__plugin_gortex_gortex__edit_file', { path: 'crates/htui/src/a.rs', old_string: 'x', new_string: NAMED }],
  ]
  const got = await verdicts($, on, calls)
  calls.forEach((c, i) => expect(got[i], c[0]).toMatch(/^htui-dev: crates\/\S+ would contain/))
})

test("Serena's replace_in_files is checked over the folder it reaches", async ($, on) => {
  const many = (input: object): Call => ['mcp__serena__replace_in_files', { needle: 'x', repl: NAMED, mode: 'literal', ...input }]
  const got = await verdicts($, on, [
    many({}),
    many({ relative_path: '' }),
    many({ relative_path: '.' }),
    many({ relative_path: 'crates' }),
    many({ relative_path: 'crates/htui/src' }),
    many({ relative_path: 'crates/htui/src/a.rs' }),
    many({ relative_path: 'docs' }),
    many({ relative_path: 'crates/htui/README.md' }),
    many({ relative_path: 'crates/htui/target' }),
    many({ dry_run: true }),
    ['mcp__serena__replace_in_files', { needle: 'x', repl: 'y', mode: 'literal' }],
  ])
  expect(got.map(r => r !== undefined)).toEqual([true, true, true, true, true, true, false, false, false, false, false])
  expect(got[0]).toBe(
    `htui-dev: replacing across the whole project would write "${LIT}" into crates/**/*.{rs,json}, which crates/htui-agent/tests/extensibility.rs forbids — use another name, or narrow relative_path to files the scan does not read`,
  )
})

test('Serena writes the scan does not read, and its read tools, are allowed', async ($, on) => {
  const got = await verdicts($, on, [
    ['mcp__serena__replace_content', { relative_path: 'docs/a.md', needle: 'x', repl: NAMED, mode: 'literal' }],
    ['mcp__serena__create_text_file', { relative_path: 'crates/htui/README.md', content: NAMED }],
    ['mcp__serena__delete_lines', { relative_path: 'crates/htui/src/a.rs', start_line: 1, end_line: 2 }],
    ['mcp__serena__safe_delete_symbol', { name_path_pattern: 'row', relative_path: 'crates/htui/src/a.rs' }],
    ['mcp__serena__find_symbol', { name_path_pattern: LIT, relative_path: 'crates/htui/src/a.rs', body: NAMED }],
  ])
  expect(got).toEqual([undefined, undefined, undefined, undefined, undefined])
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

test('a path is read by its segments, with . and .. resolved', async ($, on) => {
  const write = (file_path: string): Call => ['Write', { file_path, content: NAMED }]
  const got = await verdicts($, on, [
    write('/r/crates/htui/tests/mycrates/a.rs'),
    write('/r/crates/htui/target/../src/a.rs'),
    write('crates/htui/./src/a.rs'),
    write('/r/crates/htui/src/../target/a.rs'),
    write('crates/htui-agent/tests/../tests/extensibility.rs'),
    write('/r/mycrates/a.rs'),
  ])
  expect(got.map(r => r !== undefined)).toEqual([true, true, true, false, false, false])
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

import { test, expect } from 'claude-code/testing'
import type { On } from 'claude-code'
import type { Engine } from 'claude-code/testing'
import { bash, statuses, tools, type Reached } from './fixtures'

/** Runs each command and answers the deny reason, or undefined when the call reached the tools beneath. */
const verdicts = async ($: Engine, on: On, commands: readonly string[]) => {
  const reached: Reached[] = []
  tools(on, reached)
  const out: (string | undefined)[] = []
  for (const command of commands) {
    const before = reached.length
    const r = (await bash($, command)) as { deny?: string }
    expect(reached.length, command).toBe(r.deny === undefined ? before + 1 : before)
    out.push(r.deny)
  }
  return out
}

/** Asserts that every command is refused. */
const denies = async ($: Engine, on: On, commands: readonly string[]) => {
  const got = await verdicts($, on, commands)
  commands.forEach((c, i) => expect(got[i], c).toMatch(/^htui-dev: /))
}

/** Asserts that every command goes through. */
const allows = async ($: Engine, on: On, commands: readonly string[]) => {
  const got = await verdicts($, on, commands)
  commands.forEach((c, i) => expect(got[i], c).toBeUndefined())
}

test('each spelling of the htui selection without testkit is refused', async ($, on) => {
  await denies($, on, [
    'cargo test -p htui',
    'cargo test --package htui',
    'cargo test --package=htui',
    'cargo test -phtui',
    'cargo test --manifest-path crates/htui/Cargo.toml',
    'cargo test --manifest-path=/home/me/htui/crates/htui/Cargo.toml',
    'cargo test -p htui --features demo',
    'cargo test -p htui --no-default-features',
  ])
})

test('agent and core alone without test-support are refused', async ($, on) => {
  await denies($, on, [
    'cargo test -p htui-agent',
    'cargo test -p htui-core --features demo',
    'cargo test --manifest-path crates/htui-agent/Cargo.toml',
    'cargo test -p htui-agent -p htui-core',
    'cargo test -p htui-agent -p htui-secrets',
  ])
})

test('the whole workspace without testkit is refused', async ($, on) => {
  await denies($, on, [
    'cargo test',
    'cargo test --workspace',
    'cargo test --all',
    'cargo test --workspace --features test-support',
    'cargo test --workspace --features htui-agent/test-support',
    'cargo test --manifest-path Cargo.toml',
    'cargo test --manifest-path /r/htui/Cargo.toml',
  ])
})

// Cargo turns features on beyond the command line: a selected crate's [dependencies] and
// [dev-dependencies] (htui, orch, worker and mcp ask for agent/core/store `test-support`; store asks
// for its own), and each feature's forwards (`testkit` -> store, agent -> core, orch -> core and agent).
test("a selected crate's dependencies turn the gate on", async ($, on) => {
  await allows($, on, [
    'cargo test -p htui-store',
    'cargo test -p htui-store --test pg_store',
    'cargo test --manifest-path crates/htui-store/Cargo.toml',
    'cargo test -p htui-agent -p htui-orch',
    'cargo test -p htui-core -p htui-worker',
    'cargo test -p htui-agent -p htui-core -p htui-mcp',
    'cargo test -p htui-core -p htui-store',
    'cargo test --workspace --features testkit',
    'cargo test --features testkit',
    'cargo test -p htui -p htui-agent --features testkit',
    'cargo test -p htui-agent -p htui-core --features htui-agent/test-support',
    'cargo test -p htui-agent -p htui-core --features test-support',
    'cargo test --workspace --exclude htui',
    'cd crates/htui && cargo test --features testkit',
  ])
})

test('a manifest outside a crates folder is the workspace root, whatever its folder is called', async ($, on) => {
  const got = await verdicts($, on, [
    'cargo test --manifest-path /r/.claude/worktrees/tool-8/Cargo.toml',
    'cargo test --manifest-path=/home/me/htui-wt/Cargo.toml',
    'cargo test --manifest-path /home/me/htui/Cargo.toml',
    'cargo test --manifest-path /r/.claude/worktrees/tool-8/Cargo.toml --features testkit',
    'cargo test --manifest-path ../htui/Cargo.toml',
    'cargo test --manifest-path htui-agent/Cargo.toml',
    'cargo test --manifest-path /r/.claude/worktrees/tool-8/crates/htui-store/Cargo.toml',
  ])
  expect(got.map(r => r?.match(/builds (\S+)'s integration tests/)?.[1] ?? r)).toEqual([
    'htui',
    'htui',
    'htui',
    undefined,
    'htui',
    'htui-agent',
    undefined,
  ])
})

test('a package in a short-flag cluster, as a glob or with a version is read as cargo reads it', async ($, on) => {
  const got = await verdicts($, on, [
    'cargo test -qp htui',
    'cargo test -vvp htui-agent',
    'cargo test -qphtui',
    'cargo test -p htui@0.1.0',
    'cargo test --package=htui-core@0.1.0',
    `cargo test -p 'htui-[ac]*'`,
    `cargo test -p 'ht*'`,
    'cargo test -qp htui --features testkit',
    'cargo test -qp htui -qF testkit',
    'cargo test -p htui@0.1.0 --features testkit',
    `cargo test -p 'htui-*'`,
    `cargo test --workspace --exclude 'htui-*' --features testkit`,
    `cargo test --workspace --exclude 'h?ui'`,
  ])
  expect(got.map(r => r !== undefined)).toEqual([
    true,
    true,
    true,
    true,
    true,
    true,
    true,
    false,
    false,
    false,
    false,
    false,
    false,
  ])
})

test('integration tests in scope count, whatever selects them', async ($, on) => {
  await denies($, on, [
    'cargo test -p htui --test app',
    'cargo test -p htui --test=app',
    'cargo test -p htui --tests',
    'cargo test -p htui --all-targets',
    'cargo test -p htui --lib --tests',
    'cargo test -p htui some_filter',
    // After `--` a flag is the test binary's, not a target selector.
    'cargo test -p htui -- --lib',
  ])
})

test('every way of reaching cargo test is seen', async ($, on) => {
  await denies($, on, [
    'cargo t -p htui',
    'cargo +nightly test -p htui',
    'cargo -q --locked --color always test -p htui',
    'cargo --config net.offline=true -Z unstable-options test -p htui',
    'RUST_LOG=x timeout 900 cargo test -p htui',
    `bash -c 'cd /r && cargo test -p htui 2>&1 | tail -5'`,
    'cargo check && cargo test -p htui',
    'echo "$(cargo test -p htui 2>&1 | tail -3)"',
    '/usr/bin/cargo test -p htui',
  ])
})

test('one crate of several missing its feature is refused', async ($, on) => {
  await denies($, on, [
    'cargo test -p htui-agent -p htui-core --features htui-core/test-support',
    'cargo test -p htui -p htui-core --features htui-core/test-support',
    'cargo test -p htui -p htui-orch --features test-support',
    // store's dev-dependencies cover core, not agent.
    'cargo test -p htui-agent -p htui-core -p htui-store',
  ])
})

test('every feature spelling that enables the gate is allowed', async ($, on) => {
  await allows($, on, [
    'cargo test -p htui --features testkit',
    'cargo test -p htui --features=testkit',
    'cargo test -p htui -F testkit',
    'cargo test -p htui -Ftestkit',
    'cargo test -p htui --features "demo testkit"',
    'cargo test -p htui --features demo,testkit',
    'cargo test -p htui --features demo --features testkit',
    'cargo test -p htui --features htui/testkit',
    'cargo test -p htui --all-features',
    'cargo test -p htui-agent --features test-support',
    'cargo test -p htui-core -F htui-core/test-support',
    'cargo test -p htui-store --all-features --test pg_store -- --test-threads=1',
    'cargo test -p htui -p htui-agent --features testkit,test-support',
    'cargo test --manifest-path crates/htui/Cargo.toml --features testkit',
  ])
})

test('the whole workspace with every gating feature is allowed', async ($, on) => {
  await allows($, on, [
    'cargo test --workspace --all-features',
    'cargo test --all-features',
    'cargo test --workspace --features htui/testkit,test-support',
    'cargo test --workspace --features testkit,test-support',
    'cargo test --workspace --features htui/testkit,htui-agent/test-support,htui-core/test-support,htui-store/test-support',
    'cargo test --workspace --exclude htui --features test-support',
  ])
})

test('a run with no integration tests in scope is allowed', async ($, on) => {
  await allows($, on, [
    'cargo test -p htui --lib',
    'cargo test -p htui --bins',
    'cargo test -p htui --bin htui',
    'cargo test -p htui --doc',
    'cargo test -p htui --examples',
    'cargo test -p htui --example demo',
    'cargo test -p htui --lib --doc',
    'cargo test -p htui --bench render',
    'cargo test --workspace --benches',
    'cargo test --lib',
  ])
})

test('crates with no gated integration tests are allowed', async ($, on) => {
  await allows($, on, [
    'cargo test -p htui-mcp',
    'cargo test -p htui-secrets',
    'cargo test -p htui-orch',
    'cargo test -p htui-worker',
    'cargo test -p htui-orch --features test-support',
  ])
})

test('other cargo commands and mere mentions are allowed', async ($, on) => {
  await allows($, on, [
    'cargo build -p htui',
    'cargo clippy --workspace --all-targets',
    'cargo check -p htui --tests',
    'cargo nextest run -p htui',
    'echo cargo test -p htui',
    'git commit -m "cargo test -p htui"',
    `git commit -m "$(cat <<'EOF'\nfix: "a && cargo test -p htui" ran 0 tests\nEOF\n)"`,
    `grep -n 'cargo test' docs/htui-cli.md`,
    'cat <<EOF\ncargo test -p htui\nEOF',
  ])
})

test('a line the parser cannot follow is allowed without a status', async ($, on) => {
  const lines: (string | undefined)[] = []
  statuses(on, lines)
  await allows($, on, [`cargo test -p htui 'unbalanced`])
  expect(lines).toEqual([])
})

test('a wrong feature name is refused with the right one', async ($, on) => {
  const got = await verdicts($, on, [
    'cargo test -p htui-agent --features testkit',
    'cargo test -p htui --features test-support',
    'cargo test -p htui-orch --features testkit -- --nocapture',
    'cargo test -p htui-store --features demo,htui-store/testkit --lib',
  ])
  expect(got).toEqual([
    'htui-dev: htui-agent has no `testkit` feature (cargo will refuse it) — it is `test-support`: cargo test -p htui-agent --features test-support',
    'htui-dev: htui has no `test-support` feature (cargo will refuse it) — it is `testkit`: cargo test -p htui --features testkit',
    'htui-dev: htui-orch has no `testkit` feature (cargo will refuse it) — it is `test-support`: cargo test -p htui-orch --features test-support -- --nocapture',
    'htui-dev: htui-store has no `testkit` feature (cargo will refuse it) — it is `test-support`: cargo test -p htui-store --features demo,htui-store/test-support --lib',
  ])
})

test('a refusal names the command and the fix', async ($, on) => {
  const got = await verdicts($, on, [
    'RUST_LOG=x cargo test -p htui --test app 2>&1 | tail',
    'cargo test -p htui-core -- --test-threads=1',
    'cargo test',
    'cargo test --workspace -- --nocapture',
    'cargo test -p htui-agent -p htui-core',
  ])
  expect(got).toEqual([
    "htui-dev: `cargo test -p htui --test app` builds htui's integration tests without `testkit`, so they run 0 tests and pass — run: cargo test -p htui --test app --features testkit",
    "htui-dev: `cargo test -p htui-core -- --test-threads=1` builds htui-core's integration tests without `test-support`, so they run 0 tests and pass — run: cargo test -p htui-core --features test-support -- --test-threads=1",
    "htui-dev: `cargo test` builds htui's integration tests without `testkit`, so they run 0 tests and pass — run: cargo test --features testkit (or --all-features)",
    "htui-dev: `cargo test --workspace -- --nocapture` builds htui's integration tests without `testkit`, so they run 0 tests and pass — run: cargo test --workspace --features testkit -- --nocapture (or --all-features)",
    'htui-dev: `cargo test -p htui-agent -p htui-core` builds the integration tests of htui-agent and htui-core without `test-support`, so they run 0 tests and pass — run: cargo test -p htui-agent -p htui-core --features test-support (or --all-features)',
  ])
})

test('a cargo refusal sets no status line', async ($, on) => {
  const lines: (string | undefined)[] = []
  statuses(on, lines)
  const got = await verdicts($, on, ['cargo test -p htui', 'cargo test -p htui --features testkit'])
  expect(got[0]).toMatch(/^htui-dev: /)
  expect(got[1]).toBeUndefined()
  expect(lines).toEqual([])
})

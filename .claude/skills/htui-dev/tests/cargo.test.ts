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

test('agent, core and store without test-support are refused', async ($, on) => {
  await denies($, on, [
    'cargo test -p htui-agent',
    'cargo test -p htui-core --features demo',
    'cargo test -p htui-store --test pg_store',
    'cargo test --manifest-path crates/htui-store/Cargo.toml',
  ])
})

test('the whole workspace without every gating feature is refused', async ($, on) => {
  await denies($, on, [
    'cargo test',
    'cargo test --workspace',
    'cargo test --all',
    'cargo test --workspace --features testkit',
    'cargo test --workspace --features test-support',
    'cargo test --manifest-path Cargo.toml',
    'cargo test --manifest-path /r/htui/Cargo.toml --features testkit',
    'cd crates/htui && cargo test --features testkit',
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
    '/usr/bin/cargo test -p htui',
  ])
})

test('one crate of several missing its feature is refused', async ($, on) => {
  await denies($, on, [
    'cargo test -p htui -p htui-agent --features testkit',
    'cargo test -p htui-agent -p htui-core --features htui-agent/test-support',
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
    'cargo test --workspace --features testkit',
    'cargo test -p htui -p htui-agent --features testkit',
  ])
  expect(got).toEqual([
    "htui-dev: `cargo test -p htui --test app` builds htui's integration tests without `testkit`, so they run 0 tests and pass — run: cargo test -p htui --test app --features testkit",
    "htui-dev: `cargo test -p htui-core -- --test-threads=1` builds htui-core's integration tests without `test-support`, so they run 0 tests and pass — run: cargo test -p htui-core --features test-support -- --test-threads=1",
    'htui-dev: `cargo test --workspace --features testkit` runs the feature-gated integration tests of htui-agent, htui-core, htui-store as 0 tests — run: cargo test --workspace --features testkit --all-features',
    'htui-dev: `cargo test -p htui -p htui-agent --features testkit` runs the feature-gated integration tests of htui-agent as 0 tests — run: cargo test -p htui -p htui-agent --features testkit --all-features',
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

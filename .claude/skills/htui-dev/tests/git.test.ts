import { test, expect } from 'claude-code/testing'
import type { On } from 'claude-code'
import type { Engine } from 'claude-code/testing'
import { gitWrite } from '../hooks/guards'
import { simpleCommands } from '../hooks/shell'
import { bash, tools, type Reached } from './fixtures'

/** Runs each command from the main loop and answers the deny reason, or undefined when it went through. */
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

test('git commit with --no-verify or -n is refused', async ($, on) => {
  const commands = [
    'git commit --no-verify -m x',
    'git commit -n -m x',
    'git commit -an -m x',
    'git commit -nm x',
    'git commit --no-ver -m x',
    'git -C crates commit -n -m x',
    'git -c core.hooksPath=x commit -n -m x',
    'git --git-dir=.git --work-tree . commit -n -m x',
    'git --no-pager commit -n -m x',
    'git add . && git commit --no-verify -m x',
    `bash -c 'git commit --no-verify -m x'`,
    'echo $(git commit -n -m x)',
    'echo "$(git commit -n -m x)"',
    'git commit --verify --no-verify -m x',
    'git commit -m x --no-verify -- src/a.rs',
  ]
  const got = await verdicts($, on, commands)
  commands.forEach((c, i) => expect(got[i], c).toMatch(/^htui-dev: git commit --no-verify skips/))
})

test('a commit that runs its hooks is allowed', async ($, on) => {
  const commands = [
    'git commit -m x',
    'git commit -mn',
    'git commit -m -n',
    'git commit -m "fix -n handling"',
    'git commit -m "do not use --no-verify"',
    `git commit -F - <<'EOF'\nfix --no-verify handling\n-n\nEOF`,
    `git commit -m "$(cat <<'EOF'\nnever --no-verify\nEOF\n)"`,
    `git commit -m "$(cat <<'EOF'\nfix: handle "git log -n 3" in the parser\nEOF\n)"`,
    `git commit -m "$(cat <<'EOF'\nfix: never "git commit -n" again\nEOF\n)"`,
    'git commit --no-verify --verify -m x',
    'git commit -n --verify -m x',
    'git commit -m x -- -n',
    // The value of -C (reuse a commit's message) and of --author, not the flag.
    'git commit -Cn',
    'git commit --author -n -m x',
  ]
  const got = await verdicts($, on, commands)
  got.forEach((r, i) => expect(r, commands[i]).toBeUndefined())
})

test('-n after a value-taking option is still the flag', async ($, on) => {
  const got = await verdicts($, on, ['git commit -C HEAD -n', 'git commit --author me -n', 'git commit -m x -n'])
  got.forEach(r => expect(r).toMatch(/^htui-dev: /))
})

test('other git commands may say --no-verify or -n', async ($, on) => {
  const got = await verdicts($, on, [
    'git log -n 3',
    'git push --no-verify',
    'git merge --no-verify main',
    'git rebase -n main',
    'git config commit.verify -n',
    'echo git commit --no-verify',
  ])
  got.forEach(r => expect(r).toBeUndefined())
})

test('the refusal shows the commit without the flag', async ($, on) => {
  const got = await verdicts($, on, [
    'git commit -an -m x',
    'git commit -n -m "two words"',
    'git commit -nm x',
    'git -C d commit --no-verify -m x && git push',
    'git commit -anS -m x',
  ])
  expect(got).toEqual([
    "htui-dev: git commit --no-verify skips the repo's commit hooks — commit without it: git commit -a -m x",
    "htui-dev: git commit --no-verify skips the repo's commit hooks — commit without it: git commit -m 'two words'",
    "htui-dev: git commit --no-verify skips the repo's commit hooks — commit without it: git commit -m x",
    "htui-dev: git commit --no-verify skips the repo's commit hooks — commit without it: git -C d commit -m x",
    "htui-dev: git commit --no-verify skips the repo's commit hooks — commit without it: git commit -aS -m x",
  ])
})

test('the refusal of a heredoc commit gives back a command that parses', async ($, on) => {
  const message = `$(cat <<'EOF'\nfix: "quoted" words\nEOF\n)`
  const [got] = await verdicts($, on, [`git commit -n -m "${message}"`])
  const fixed = got!.split('commit without it: ')[1]!
  expect(simpleCommands(fixed)).toEqual([['git', 'commit', '-m', message]])
})

test('gitWrite names the git commands that change the tree or history', () => {
  const table: [string, string | null][] = [
    ['git commit -m x', 'git commit'],
    ['git -C d commit --amend', 'git commit'],
    ['git stash', 'git stash'],
    ['git stash pop', 'git stash'],
    ['git stash push -m x', 'git stash'],
    ['git stash drop', 'git stash'],
    ['git reset --hard', 'git reset'],
    ['git reset HEAD~1', 'git reset'],
    ['git checkout -- src/a.rs', 'git checkout --'],
    ['git checkout HEAD -- src/a.rs', 'git checkout --'],
    ['git stash list', null],
    ['git stash show -p', null],
    ['git status', null],
    ['git diff HEAD', null],
    ['git log --oneline', null],
    ['git checkout main', null],
    ['git checkout -b topic', null],
    ['echo git commit', null],
    ['cargo test', null],
  ]
  for (const [line, label] of table) expect(gitWrite(simpleCommands(line)![0]!), line).toBe(label)
})

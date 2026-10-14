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
    // git 2.43 takes --no-veri; --no-ver is ambiguous with --no-verbose.
    'git commit --no-veri -m x',
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
    // An abbreviated value-taking option takes the next word as git does.
    'git commit --mess -n',
    'git commit --auth -n -m x',
    'git commit --templ -n',
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

test('gitWrite names the git commands that change the tree, the index or history', () => {
  const table: [string, string | null][] = [
    ['git commit -m x', 'git commit'],
    ['git -C d commit --amend', 'git commit'],
    ['git stash', 'git stash'],
    ['git stash pop', 'git stash'],
    ['git stash push -m x', 'git stash'],
    ['git stash drop', 'git stash'],
    ['git reset --hard', 'git reset'],
    ['git reset HEAD~1', 'git reset'],
    ['git checkout -- src/a.rs', 'git checkout <paths>'],
    ['git checkout HEAD -- src/a.rs', 'git checkout <paths>'],
    ['git checkout .', 'git checkout <paths>'],
    ['git -C /r checkout .', 'git checkout <paths>'],
    ['git checkout HEAD~1 src/a.rs', 'git checkout <paths>'],
    ['git checkout --pathspec-from-file=f HEAD', 'git checkout <paths>'],
    ['git checkout -f main', 'git checkout --force'],
    ['git checkout --force', 'git checkout --force'],
    ['git checkout -p', 'git checkout --patch'],
    ['git restore .', 'git restore'],
    ['git restore --staged src/a.rs', 'git restore'],
    ['git clean -fdx', 'git clean'],
    ['git merge main', 'git merge'],
    ['git merge --abort', 'git merge'],
    ['git rebase main', 'git rebase'],
    ['git cherry-pick abc', 'git cherry-pick'],
    ['git revert abc', 'git revert'],
    ['git am x.patch', 'git am'],
    ['git apply x.patch', 'git apply'],
    ['git add -A', 'git add'],
    ['git rm src/a.rs', 'git rm'],
    ['git mv a b', 'git mv'],
    ['git pull', 'git pull'],
    ['git branch topic', 'git branch'],
    ['git branch -d topic', 'git branch'],
    ['git branch -D topic', 'git branch'],
    ['git branch -m a b', 'git branch'],
    ['git branch --delete topic', 'git branch'],
    ['git branch -f topic HEAD', 'git branch'],
    ['git branch -u origin/x', 'git branch'],
    ['git tag v1', 'git tag'],
    ['git tag -a v1 -m x', 'git tag'],
    ['git tag -d v1', 'git tag'],
    ['git stash list', null],
    ['git stash show -p', null],
    ['git status', null],
    ['git diff HEAD', null],
    ['git log --oneline', null],
    ['git show HEAD', null],
    ['git blame src/a.rs', null],
    ['git fetch origin', null],
    ['git checkout main', null],
    ['git checkout -b topic', null],
    ['git checkout -b topic origin/main', null],
    ['git clean -n', null],
    ['git clean -nd', null],
    ['git clean --dry-run', null],
    ['git branch', null],
    ['git branch -a', null],
    ['git branch -vv', null],
    ['git branch --list "tool-*"', null],
    ['git branch --contains abc', null],
    ['git branch --merged main', null],
    ['git branch --show-current', null],
    ['git tag', null],
    ['git tag -l "v*"', null],
    ['git tag --list', null],
    ['git tag -n5', null],
    ['git tag --contains abc', null],
    ['git tag -v v1', null],
    ['echo git commit', null],
    ['cargo test', null],
  ]
  for (const [line, label] of table) expect(gitWrite(simpleCommands(line)![0]!), line).toBe(label)
})

test('git switch mirrors git checkout: only a force, a force-create or an orphan writes', () => {
  const table: [string, string | null][] = [
    ['git switch -f main', 'git switch --force'],
    ['git switch --force main', 'git switch --force'],
    ['git switch -qf main', 'git switch --force'],
    ['git switch --discard-changes main', 'git switch --discard-changes'],
    ['git switch --disc main', 'git switch --discard-changes'],
    ['git switch -C topic', 'git switch --force-create'],
    ['git switch --force-create topic', 'git switch --force-create'],
    ['git switch --force-c topic', 'git switch --force-create'],
    ['git switch --orphan x', 'git switch --orphan'],
    ['git switch --orph x', 'git switch --orphan'],
    ['git switch main', null],
    ['git switch -', null],
    ['git switch -c topic', null],
    ['git switch -c topic origin/main', null],
    ['git switch -cf', null],
    ['git switch -c f', null],
    ['git switch --create topic', null],
    ['git switch --detach HEAD~1', null],
    ['git switch --det HEAD~1', null],
    ['git switch -t origin/x', null],
    ['git switch --no-force main', null],
    // Ambiguous: git refuses `--fo` (force or force-create) and `--d` (discard-changes or detach).
    ['git switch --fo main', null],
    ['git switch --d main', null],
  ]
  for (const [line, label] of table) expect(gitWrite(simpleCommands(line)![0]!), line).toBe(label)
})

test('git checkout reads --ours/--theirs as paths and long options by their unique prefix', () => {
  const table: [string, string | null][] = [
    ['git checkout --ours a.rs', 'git checkout <paths>'],
    ['git checkout --theirs a.rs', 'git checkout <paths>'],
    ['git checkout --ou a.rs', 'git checkout <paths>'],
    ['git checkout -2 a.rs', 'git checkout <paths>'],
    ['git checkout -3 a.rs', 'git checkout <paths>'],
    ['git checkout --forc main', 'git checkout --force'],
    ['git checkout --patc', 'git checkout --patch'],
    ['git checkout -qf main', 'git checkout --force'],
    ['git checkout --pathspec-fr=f', 'git checkout <paths>'],
    ['git checkout -b f', null],
    ['git checkout -bf', null],
    ['git checkout -B f origin/f', null],
    ['git checkout --orphan f', null],
    ['git checkout --det HEAD', null],
    ['git checkout -q --detach main', null],
    ['git checkout -t origin/x', null],
    ['git checkout --no-force main', null],
    ['git checkout -', null],
    // Ambiguous: git refuses `--pat` (pathspec-from-file or pathspec-file-nul).
    ['git checkout --pat', null],
  ]
  for (const [line, label] of table) expect(gitWrite(simpleCommands(line)![0]!), line).toBe(label)
})

test('git clean is a dry run only with -n or a prefix of --dry-run, before -- and outside a value', () => {
  const table: [string, string | null][] = [
    ['git clean -fden', 'git clean'],
    ['git clean -fdx -- -n', 'git clean'],
    ['git clean -fd -e n', 'git clean'],
    ['git clean -e -n -fd', 'git clean'],
    ['git clean --exclude -n -fd', 'git clean'],
    ['git clean --exclude=-n -f', 'git clean'],
    ['git clean -f', 'git clean'],
    ['git clean --dry -fd', null],
    ['git clean --dr', null],
    ['git clean -fdn', null],
    ['git clean -nfd', null],
    ['git clean -n -e x', null],
    ['git clean -fd -n -- a', null],
  ]
  for (const [line, label] of table) expect(gitWrite(simpleCommands(line)![0]!), line).toBe(label)
})

test('gitWrite names the writes to the remote, refs, worktrees, submodules and history beyond the porcelain', () => {
  const table: [string, string | null][] = [
    ['git push', 'git push'],
    ['git push --force origin topic', 'git push'],
    ['git push --dry-run', 'git push'],
    ['git bisect start', 'git bisect'],
    ['git bisect good', 'git bisect'],
    ['git bisect reset', 'git bisect'],
    ['git worktree add ../x', 'git worktree add'],
    ['git worktree remove x', 'git worktree remove'],
    ['git worktree move x y', 'git worktree move'],
    ['git worktree prune', 'git worktree prune'],
    ['git submodule update --init', 'git submodule update'],
    ['git submodule --quiet deinit x', 'git submodule deinit'],
    ['git submodule add url x', 'git submodule add'],
    ['git submodule sync', 'git submodule sync'],
    ['git sparse-checkout set x', 'git sparse-checkout'],
    ['git sparse-checkout disable', 'git sparse-checkout'],
    ['git checkout-index -a -f', 'git checkout-index'],
    ['git read-tree -u HEAD', 'git read-tree'],
    ['git update-ref -d refs/heads/x', 'git update-ref'],
    ['git update-index --assume-unchanged a', 'git update-index'],
    ['git reflog expire --all', 'git reflog expire'],
    ['git reflog delete HEAD@{1}', 'git reflog delete'],
    ['git gc --prune=now', 'git gc'],
    ['git notes add -m x', 'git notes add'],
    ['git notes --ref r remove', 'git notes remove'],
    ['git notes --ref=r edit', 'git notes edit'],
    ['git notes append -m x', 'git notes append'],
    ['git notes copy a b', 'git notes copy'],
    ['git notes prune', 'git notes prune'],
    ['git replace a b', 'git replace'],
    ['git replace -d a', 'git replace'],
    ['git filter-branch --tree-filter x', 'git filter-branch'],
    ['git bisect log', null],
    ['git bisect view', null],
    ['git bisect visualize', null],
    ['git worktree list', null],
    ['git submodule', null],
    ['git submodule status', null],
    ['git sparse-checkout list', null],
    ['git reflog', null],
    ['git reflog show', null],
    ['git notes', null],
    ['git notes list', null],
    ['git notes show HEAD', null],
    ['git replace', null],
    ['git replace -l', null],
    ['git replace --list "x*"', null],
  ]
  for (const [line, label] of table) expect(gitWrite(simpleCommands(line)![0]!), line).toBe(label)
})

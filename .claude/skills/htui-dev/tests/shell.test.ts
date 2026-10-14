import { test, expect } from 'claude-code/testing'
import { quoteWords, simpleCommands } from '../hooks/shell'

test('each separator ends a simple command', () => {
  for (const sep of [';', '&&', '||', '|', '|&', '&', '\n']) {
    expect(simpleCommands(`a 1${sep}b 2`), sep).toEqual([['a', '1'], ['b', '2']])
  }
  expect(simpleCommands('a;b')).toEqual([['a'], ['b']])
  expect(simpleCommands('a &')).toEqual([['a']])
  expect(simpleCommands('')).toEqual([])
})

test('a separator inside quotes is text', () => {
  expect(simpleCommands(`echo 'a; b && c' "d | e"`)).toEqual([['echo', 'a; b && c', 'd | e']])
  expect(simpleCommands(`echo "it's"`)).toEqual([['echo', "it's"]])
  expect(simpleCommands(`echo a'b c'"d"`)).toEqual([['echo', 'ab cd']])
  expect(simpleCommands(`echo '' ""`)).toEqual([['echo', '', '']])
})

test('a backslash keeps the next character in the word', () => {
  expect(simpleCommands('echo a\\ b \\; c')).toEqual([['echo', 'a b', ';', 'c']])
  expect(simpleCommands(`echo \\'x\\"`)).toEqual([['echo', `'x"`]])
  expect(simpleCommands('echo "a\\"b" "c\\d"')).toEqual([['echo', 'a"b', 'c\\d']])
  expect(simpleCommands("echo 'a\\'")).toEqual([['echo', 'a\\']])
  expect(simpleCommands('cargo test \\\n  -p htui')).toEqual([['cargo', 'test', '-p', 'htui']])
})

test('redirections leave no words behind and split nothing', () => {
  expect(simpleCommands('cargo test 2>&1 | tail -5')).toEqual([['cargo', 'test'], ['tail', '-5']])
  expect(simpleCommands('cargo test &>log; echo done')).toEqual([['cargo', 'test'], ['echo', 'done']])
  expect(simpleCommands('cargo test &>> log')).toEqual([['cargo', 'test']])
  expect(simpleCommands('cargo test >out 2> err <in')).toEqual([['cargo', 'test']])
  expect(simpleCommands('cargo test >>out 2>>err 1>&- >|f')).toEqual([['cargo', 'test']])
  expect(simpleCommands('cargo test |& tee log')).toEqual([['cargo', 'test'], ['tee', 'log']])
  expect(simpleCommands('cat <<< "git commit -n"')).toEqual([['cat']])
  // A number that is not right before the operator is a word.
  expect(simpleCommands('echo 2 >f')).toEqual([['echo', '2']])
})

test('leading assignments are dropped', () => {
  expect(simpleCommands('RUST_LOG=debug A="x y" cargo test')).toEqual([['cargo', 'test']])
  expect(simpleCommands('X=1')).toEqual([])
  expect(simpleCommands('cargo test X=1')).toEqual([['cargo', 'test', 'X=1']])
})

test('each wrapper is stripped with its options, nested ones too', () => {
  const cases: [string, string[]][] = [
    ['env cargo test', ['cargo', 'test']],
    ['env -i -u HOME --unset=PATH A=1 cargo test', ['cargo', 'test']],
    ['env -uHOME -- cargo test', ['cargo', 'test']],
    ['command cargo test', ['cargo', 'test']],
    ['command -p cargo test', ['cargo', 'test']],
    ['time cargo test', ['cargo', 'test']],
    ['time -p cargo test', ['cargo', 'test']],
    ['nice cargo test', ['cargo', 'test']],
    ['nice -n 10 cargo test', ['cargo', 'test']],
    ['nice -n10 cargo test', ['cargo', 'test']],
    ['nice --adjustment=5 cargo test', ['cargo', 'test']],
    ['timeout 900 cargo test', ['cargo', 'test']],
    ['timeout -s KILL -k 5 900s cargo test', ['cargo', 'test']],
    ['timeout --signal=TERM --preserve-status 15m cargo test', ['cargo', 'test']],
    ['nohup cargo test', ['cargo', 'test']],
    ['exec cargo test', ['cargo', 'test']],
    ['exec -a name -cl cargo test', ['cargo', 'test']],
    ['RUST_LOG=x timeout 900 nice -n 5 env A=1 nohup time cargo test', ['cargo', 'test']],
  ]
  for (const [line, words] of cases) expect(simpleCommands(line), line).toEqual([words])
  expect(simpleCommands('time')).toEqual([])
})

test('env -S splits its string into the command', () => {
  expect(simpleCommands('env -S "cargo test -p htui"')).toEqual([['cargo', 'test', '-p', 'htui']])
  expect(simpleCommands(`env -S'cargo test' -p htui`)).toEqual([['cargo', 'test', '-p', 'htui']])
  expect(simpleCommands('env --split-string="cargo test" -p htui')).toEqual([['cargo', 'test', '-p', 'htui']])
  expect(simpleCommands('env -i -S "A=1 git commit -n"')).toEqual([['git', 'commit', '-n']])
  expect(simpleCommands('env -uSHELL cargo test')).toEqual([['cargo', 'test']])
})

test('an array assignment is no command', () => {
  expect(simpleCommands('arr=(cargo test -p htui)')).toEqual([])
  expect(simpleCommands('arr=(a "b c" $(x)) && cargo test')).toEqual([['cargo', 'test']])
  expect(simpleCommands('arr+=(git commit -n); ls')).toEqual([['ls']])
  expect(simpleCommands('declare -a xs=(git commit -n)')).toEqual([['declare', '-a', 'xs=(git commit -n)']])
})

test('leading shell keywords and braces are not the command', () => {
  expect(simpleCommands('if cargo test; then git commit -n; fi')).toEqual([
    ['cargo', 'test'],
    ['git', 'commit', '-n'],
  ])
  expect(simpleCommands('for c in a b; do cargo test -p $c; done')).toEqual([
    ['for', 'c', 'in', 'a', 'b'],
    ['cargo', 'test', '-p', '$c'],
  ])
  expect(simpleCommands('{ git commit -n; }')).toEqual([['git', 'commit', '-n']])
  expect(simpleCommands('! cargo test')).toEqual([['cargo', 'test']])
})

test('a subshell, a substitution and backticks are commands of their own', () => {
  expect(simpleCommands('(cd crates/htui && cargo test)')).toEqual([['cd', 'crates/htui'], ['cargo', 'test']])
  expect(simpleCommands('echo $(git commit -n) end')).toEqual([['echo'], ['git', 'commit', '-n'], ['end']])
  expect(simpleCommands('x=$(git commit -n)')).toEqual([['git', 'commit', '-n']])
  expect(simpleCommands('echo `git commit -n`')).toEqual([['echo'], ['git', 'commit', '-n']])
})

test('a bash -c string is followed', () => {
  expect(simpleCommands(`bash -c 'cd x && cargo test -p htui'`)).toEqual([['cd', 'x'], ['cargo', 'test', '-p', 'htui']])
  expect(simpleCommands(`sh -c "git commit -n"`)).toEqual([['git', 'commit', '-n']])
  expect(simpleCommands(`bash -lc 'cargo test' arg0`)).toEqual([['cargo', 'test']])
  expect(simpleCommands(`bash -e -o pipefail -c 'cargo test'`)).toEqual([['cargo', 'test']])
  expect(simpleCommands(`/bin/bash -ec "bash -c 'git commit -n'"`)).toEqual([['git', 'commit', '-n']])
  expect(simpleCommands(`env bash -c 'cargo test'`)).toEqual([['cargo', 'test']])
  expect(simpleCommands(`bash --rcfile f -c 'cargo test'`)).toEqual([['cargo', 'test']])
  expect(simpleCommands(`bash --init-file f --norc -c 'cargo test'`)).toEqual([['cargo', 'test']])
  for (const shell of ['zsh', 'dash', 'ksh', '/usr/bin/zsh']) {
    expect(simpleCommands(`${shell} -c 'git commit -n'`), shell).toEqual([['git', 'commit', '-n']])
  }
  // Not a -c: the script runs, the parser cannot see into it.
  expect(simpleCommands('bash scripts/gate.sh -c')).toEqual([['bash', 'scripts/gate.sh', '-c']])
  expect(simpleCommands(`bash -c 'echo "unbalanced'`)).toBeNull()
})

test('a heredoc body is not read as commands', () => {
  expect(simpleCommands('cat <<EOF > f\ngit commit -n\nEOF\necho done')).toEqual([['cat'], ['echo', 'done']])
  expect(simpleCommands("cat <<'EOF'\ncargo test -p htui\nEOF")).toEqual([['cat']])
  expect(simpleCommands('cat <<"EOF"\n$(git commit -n)\nEOF')).toEqual([['cat']])
  expect(simpleCommands('cat <<-EOF\n\tgit commit -n\n\tEOF\nls')).toEqual([['cat'], ['ls']])
  expect(simpleCommands('cat <<A <<B\ngit commit -n\nA\ncargo test\nB\nls')).toEqual([['cat'], ['ls']])
  expect(simpleCommands(`git commit -F - <<'EOF'\nfix --no-verify handling\nEOF`)).toEqual([
    ['git', 'commit', '-F', '-'],
  ])
})

test('a quoted heredoc inside a substitution stays in its word', () => {
  const line = `git commit -m "$(cat <<'EOF'\nfix: do not use --no-verify\n\nCo-Authored-By: x\nEOF\n)"`
  expect(simpleCommands(line)).toEqual([
    ['cat'],
    ['git', 'commit', '-m', `$(cat <<'EOF'\nfix: do not use --no-verify\n\nCo-Authored-By: x\nEOF\n)`],
  ])
})

test('a substitution inside double quotes opens quotes of its own', () => {
  // The standard commit form, with double quotes in the message.
  const message = `$(cat <<'EOF'\nfix: handle "git log -n 3" in the parser\nEOF\n)`
  expect(simpleCommands(`git commit -m "${message}"`)).toEqual([['cat'], ['git', 'commit', '-m', message]])
  const separators = `$(cat <<'EOF'\nrun "a && cargo test -p htui"; then (b)\nEOF\n)`
  expect(simpleCommands(`git commit -m "${separators}"`)).toEqual([['cat'], ['git', 'commit', '-m', separators]])
  // An odd number of inner quotes is no unbalanced line.
  const odd = `$(cat <<'EOF'\nsay "hi\nEOF\n)`
  expect(simpleCommands(`git commit -m "${odd}"`)).toEqual([['cat'], ['git', 'commit', '-m', odd]])
  expect(simpleCommands(`echo "a $(echo ")") b"`)).toEqual([['echo', ')'], ['echo', 'a $(echo ")") b']])
})

test('a substitution inside double quotes is followed as commands', () => {
  expect(simpleCommands(`echo "$(git commit -n)"`)).toEqual([['git', 'commit', '-n'], ['echo', '$(git commit -n)']])
  expect(simpleCommands(`x="$(cd a && cargo test -p htui)"`)).toEqual([['cd', 'a'], ['cargo', 'test', '-p', 'htui']])
  expect(simpleCommands(`echo "$(echo "$(git commit -n)")"`)).toEqual([
    ['git', 'commit', '-n'],
    ['echo', '$(git commit -n)'],
    ['echo', '$(echo "$(git commit -n)")'],
  ])
  // Arithmetic is no command, and an escaped dollar is text.
  expect(simpleCommands(`echo "$((1<<2))" "\\$(git commit -n)"`)).toEqual([['echo', '$((1<<2))', '$(git commit -n)']])
  expect(simpleCommands(`echo "$(git commit -n"`)).toBeNull()
})

test('a comment is skipped', () => {
  expect(simpleCommands('cargo check # then git commit -n\nls')).toEqual([['cargo', 'check'], ['ls']])
  expect(simpleCommands('# git commit -n')).toEqual([])
  expect(simpleCommands('echo a#b')).toEqual([['echo', 'a#b']])
})

test('a line the parser cannot follow answers null', () => {
  expect(simpleCommands(`echo 'abc`)).toBeNull()
  expect(simpleCommands(`echo "abc`)).toBeNull()
  expect(simpleCommands('echo abc\\')).toBeNull()
  expect(simpleCommands('cat <<EOF\ngit commit -n')).toBeNull()
  expect(simpleCommands('cat <<')).toBeNull()
})

test('quoteWords survives a round trip through simpleCommands', () => {
  const words = ['git', 'commit', '-m', "it's a b", '', 'x;y', '$HOME', 'a\\b', 'plain-word_1.rs', '--features=a,b']
  expect(simpleCommands(quoteWords(words))).toEqual([words])
  expect(quoteWords(['cargo', 'test', '-p', 'htui'])).toBe('cargo test -p htui')
  expect(quoteWords(['git', 'commit', '-m', 'two words'])).toBe(`git commit -m 'two words'`)
})

// The guards' decisions, pure: each reads plain words or a plain record and answers the deny
// reason (`htui-dev: <what> — <the fix>`), or null to let the call through. Nothing here calls
// `$`; register.ts wires them onto the events and catches what they throw.
import { quoteWords, type Words } from './shell'

// --- G1 and G2: cargo test -------------------------------------------------------------------

// The three tables below are copied from the manifests; a drift is TOOL-9's gate to catch
// (`cargo tree -e features` shows what cargo really turns on). Every feature is `<crate>/<name>`.

/**
 * The features each member declares, and what each forwards to: the `[features]` tables of
 * `crates/*\/Cargo.toml`. Only the forwards that reach a test feature are listed (`dep:` ones are not).
 */
const FEATURES: Readonly<Record<string, Readonly<Record<string, readonly string[]>>>> = {
  htui: { testkit: ['htui-store/test-support'] },
  'htui-agent': { 'test-support': ['htui-core/test-support'] },
  'htui-core': { demo: [], 'test-support': ['htui-core/demo'], sqlx: [] },
  'htui-mcp': {},
  'htui-orch': { 'test-support': ['htui-core/test-support', 'htui-agent/test-support'] },
  'htui-secrets': {},
  'htui-store': { demo: ['htui-core/demo'], 'test-support': ['htui-store/demo'], 'local-embed': [] },
  'htui-worker': { 'test-support': [] },
}

/**
 * What selecting a member for `cargo test` turns on by itself: the `features = [...]` of its
 * `[dependencies]` and, since a test build uses them, its `[dev-dependencies]` (only `demo` and
 * `test-support` listed). Cargo unifies them over every selected crate, so `-p htui-agent
 * -p htui-orch` builds the agent's own tests with `test-support`, and `-p htui-store` its own.
 */
const SELECTING: Readonly<Record<string, readonly string[]>> = {
  htui: [
    'htui-core/demo',
    'htui-store/demo',
    'htui-agent/test-support',
    'htui-orch/test-support',
    'htui-worker/test-support',
    'htui-store/test-support',
  ],
  'htui-orch': ['htui-core/test-support', 'htui-agent/test-support'],
  'htui-worker': [
    'htui-core/test-support',
    'htui-orch/test-support',
    'htui-agent/test-support',
    'htui-store/demo',
    'htui-store/test-support',
  ],
  'htui-mcp': [
    'htui-core/test-support',
    'htui-agent/test-support',
    'htui-orch/test-support',
    'htui-store/demo',
    'htui-store/test-support',
  ],
  'htui-store': ['htui-core/test-support', 'htui-store/test-support'],
}

/**
 * The features a member's integration tests are gated on (`#![cfg(feature = …)]` over
 * `crates/*\/tests`: htui 43 files on `testkit`; agent 4 and core 8 on `test-support`; store 3 on
 * `test-support` and 5 on `demo`). The first is the one a refusal suggests.
 */
const GATES: Readonly<Record<string, readonly string[]>> = {
  htui: ['testkit'],
  'htui-agent': ['test-support'],
  'htui-core': ['test-support'],
  'htui-store': ['test-support', 'demo'],
}

/** Every member's test feature, `testkit` or `test-support`, gated tests or not: G2 checks the spelling. */
const TEST_FEATURE: Readonly<Record<string, string>> = Object.fromEntries(
  Object.entries(FEATURES).flatMap(([crate, own]) =>
    ['testkit', 'test-support'].filter(f => own[f] !== undefined).map(f => [crate, f]),
  ),
)

/** The virtual root's members (`Cargo.toml` `[workspace]`): a run with no package selection tests them all. */
const MEMBERS = [
  'htui',
  'htui-agent',
  'htui-core',
  'htui-mcp',
  'htui-orch',
  'htui-secrets',
  'htui-store',
  'htui-worker',
] as const

/** The two test feature names; a third spelling is no business of G2's. */
const FEATURE_NAMES = new Set(['testkit', 'test-support'])

/** cargo's own options before the subcommand that take a value as the next word. */
const CARGO_GLOBAL_VALUE = new Set(['--color', '--config', '-Z', '-C', '--explain'])

/** `cargo test` options that take a value as the next word (beside the ones read below). */
const TEST_VALUE = new Set([
  '--target',
  '--target-dir',
  '-j',
  '--jobs',
  '--profile',
  '--color',
  '--config',
  '-Z',
  '--message-format',
  '--lockfile-path',
])

/** Target selectors that leave the integration tests out of the run. */
const OUT_OF_SCOPE = new Set(['--lib', '--bin', '--bins', '--doc', '--example', '--examples', '--bench', '--benches'])

/** Target selectors that bring them in. */
const IN_SCOPE = new Set(['--test', '--tests', '--all-targets'])

/** One `--features` value as written: the word it sits in and the text before it there (`--features=`, `-F`, or none). */
type FeatureWord = { index: number; prefix: string }

/** What a `cargo test` command asks for, read from its words. */
type CargoTest = {
  words: Words
  /** Selected packages (`-p`, `--manifest-path`), or null for the whole workspace. */
  packages: string[] | null
  excluded: string[]
  features: string[]
  featureWords: FeatureWord[]
  allFeatures: boolean
  /** Whether the run builds the integration tests (`--test`, `--tests`, `--all-targets`, or no selector). */
  inScope: boolean
  /** Where the test binary's own arguments start (`--`), or the words' length. */
  dashDash: number
}

/**
 * The package a `--manifest-path` names: its folder's name when that folder sits in `crates/` (or
 * `../`), or is a bare `<member>/Cargo.toml`; otherwise null, the workspace root, whatever its
 * folder is called (`/r/.claude/worktrees/tool-8/Cargo.toml`, `/home/me/htui/Cargo.toml`).
 */
function manifestPackage(path: string): string | null {
  const parts = path.replace(/\\/g, '/').split('/').filter(p => p !== '' && p !== '.')
  const name = parts[parts.length - 2]
  if (name === undefined) return null
  const above = parts[parts.length - 3]
  if (above === 'crates' || above === '..') return name
  const bare = above === undefined && !path.startsWith('/') && (MEMBERS as readonly string[]).includes(name)
  return bare ? name : null
}

/** `cargo test`'s short options that take a value, attached (`-phtui`) or as the next word. */
const SHORT_VALUE = new Set(['p', 'F', 'j', 'Z'])

/** The members a package spec names: `@version` dropped, a glob (`htui-*`) matched against the members, anything else as written. */
function packagesOf(spec: string): string[] {
  const name = spec.replace(/@.*$/, '')
  if (!/[*?[]/.test(name)) return [name]
  const pattern = name.replace(/[.+^${}()|\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')
  return MEMBERS.filter(m => new RegExp(`^${pattern}$`).test(m))
}

/** Reads `words` as a `cargo test` (or `cargo t`) command, or null when it is none. */
function parseCargoTest(words: Words): CargoTest | null {
  if ((words[0] ?? '').split('/').pop() !== 'cargo') return null
  let at = 1
  while (at < words.length) {
    const w = words[at]!
    if (w.startsWith('+')) at++
    else if (CARGO_GLOBAL_VALUE.has(w)) at += 2
    else if (w.startsWith('-')) at++
    else break
  }
  if (words[at] !== 'test' && words[at] !== 't') return null

  const got: CargoTest = {
    words,
    packages: null,
    excluded: [],
    features: [],
    featureWords: [],
    allFeatures: false,
    inScope: true,
    dashDash: words.length,
  }
  let workspace = false
  let selectors = 0
  let inScopeSelectors = 0
  const packages: string[] = []
  const addFeatures = (index: number, prefix: string) => {
    got.featureWords.push({ index, prefix })
    got.features.push(...words[index]!.slice(prefix.length).split(/[\s,]+/).filter(f => f !== ''))
  }
  /** The value of option `name` at `i`: `name=v`, `-xv` (short), or the next word; `[value, words used]`. */
  const valueOf = (i: number, name: string): [string | undefined, number] => {
    const w = words[i]!
    if (w === name) return [words[i + 1], 2]
    return [w.startsWith('--') ? w.slice(name.length + 1) : w.slice(name.length), 1]
  }
  /** Whether `w` is option `name`, alone or with its value attached. */
  const isOpt = (w: string, name: string) =>
    w === name || (name.startsWith('--') ? w.startsWith(name + '=') : w.startsWith(name))

  for (let i = at + 1; i < words.length; ) {
    const w = words[i]!
    if (w === '--') {
      got.dashDash = i
      break
    }
    if (/^-[A-Za-z]/.test(w)) {
      // A short cluster (`-qp htui`, `-vvF testkit`): flags until one takes a value, which is the
      // rest of the cluster or the next word.
      let used = 1
      for (let j = 1; j < w.length; j++) {
        const letter = w[j]!
        if (!SHORT_VALUE.has(letter)) continue
        const attached = w.slice(j + 1)
        const value = attached !== '' ? attached : words[i + 1]
        if (attached === '') used = 2
        if (letter === 'p' && value !== undefined) packages.push(...packagesOf(value))
        if (letter === 'F' && value !== undefined) {
          if (attached !== '') addFeatures(i, w.slice(0, j + 1))
          else addFeatures(i + 1, '')
        }
        break
      }
      i += used
    } else if (isOpt(w, '--package')) {
      const [v, n] = valueOf(i, '--package')
      if (v !== undefined) packages.push(...packagesOf(v))
      i += n
    } else if (isOpt(w, '--manifest-path')) {
      const [v, n] = valueOf(i, '--manifest-path')
      const pkg = v === undefined ? null : manifestPackage(v)
      if (pkg !== null) packages.push(pkg)
      i += n
    } else if (w === '--workspace' || w === '--all') {
      workspace = true
      i++
    } else if (isOpt(w, '--exclude')) {
      const [v, n] = valueOf(i, '--exclude')
      if (v !== undefined) got.excluded.push(...packagesOf(v))
      i += n
    } else if (w === '--all-features') {
      got.allFeatures = true
      i++
    } else if (isOpt(w, '--features')) {
      if (w === '--features') {
        if (i + 1 < words.length) addFeatures(i + 1, '')
        i += 2
      } else {
        addFeatures(i, '--features=')
        i++
      }
    } else if (OUT_OF_SCOPE.has(w.split('=')[0]!) || IN_SCOPE.has(w.split('=')[0]!)) {
      const name = w.split('=')[0]!
      selectors++
      if (IN_SCOPE.has(name)) inScopeSelectors++
      // `--test <name>`, `--bin <name>`, ...: the name is the next word unless attached with `=`.
      const takesName = !w.includes('=') && ['--test', '--bin', '--example', '--bench'].includes(name)
      i += takesName && i + 1 < words.length && !words[i + 1]!.startsWith('-') ? 2 : 1
    } else if (TEST_VALUE.has(w)) {
      i += 2
    } else {
      i++
    }
  }
  got.packages = workspace || packages.length === 0 ? null : packages
  got.inScope = selectors === 0 || inScopeSelectors > 0
  return got
}

/**
 * The features (`<crate>/<name>`) a run of `t` over the crates `selected` turns on: the command
 * line's (`--all-features`; a plain name on each selected crate that has it; `<crate>/<name>`), what
 * selecting each crate turns on, and everything those forward to.
 */
function enabledFeatures(t: CargoTest, selected: readonly string[]): Set<string> {
  const queue: string[] = []
  for (const crate of selected) {
    const own = FEATURES[crate] ?? {}
    if (t.allFeatures) queue.push(...Object.keys(own).map(f => `${crate}/${f}`))
    for (const term of t.features) if (!term.includes('/') && own[term] !== undefined) queue.push(`${crate}/${term}`)
    queue.push(...(SELECTING[crate] ?? []))
  }
  queue.push(...t.features.filter(term => term.includes('/')))
  const on = new Set<string>()
  while (queue.length > 0) {
    const feature = queue.pop()!
    if (on.has(feature)) continue
    on.add(feature)
    const [crate, name] = feature.split('/') as [string, string]
    queue.push(...(FEATURES[crate]?.[name] ?? []))
  }
  return on
}

/** `a`, `a and b`, `a, b and c`. */
const listed = (items: readonly string[]): string =>
  items.length < 2 ? items.join('') : `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`

/** `t`'s words with `add` put before `--`, or at the end. */
const withWords = (t: CargoTest, add: readonly string[]): Words => [
  ...t.words.slice(0, t.dashDash),
  ...add,
  ...t.words.slice(t.dashDash),
]

/** G2: a test feature named for a crate that spells it the other way, with the command corrected. */
function wrongFeature(t: CargoTest): string | null {
  const selected = t.packages ?? [...MEMBERS]
  for (const term of t.features) {
    const slash = term.indexOf('/')
    const crate = slash === -1 ? null : term.slice(0, slash)
    const name = slash === -1 ? term : term.slice(slash + 1)
    if (!FEATURE_NAMES.has(name)) continue
    let owner: string | undefined
    if (crate !== null) {
      if (TEST_FEATURE[crate] !== undefined && TEST_FEATURE[crate] !== name) owner = crate
    } else if (!selected.some(c => TEST_FEATURE[c] === name)) {
      owner = selected.find(c => TEST_FEATURE[c] !== undefined)
    }
    if (owner === undefined) continue
    const right = TEST_FEATURE[owner]!
    const fixed = t.words.map((w, i) => {
      const fw = t.featureWords.find(f => f.index === i)
      if (fw === undefined) return w
      const swapped = w
        .slice(fw.prefix.length)
        .split(/([\s,]+)/)
        .map(part => (part === term ? (crate === null ? right : `${crate}/${right}`) : part))
        .join('')
      return fw.prefix + swapped
    })
    return `htui-dev: ${owner} has no \`${name}\` feature (cargo will refuse it) — it is \`${right}\`: ${quoteWords(fixed)}`
  }
  return null
}

/** G1: integration tests built without the feature they are gated on, so they run 0 tests and pass. */
function vacuousRun(t: CargoTest): string | null {
  if (!t.inScope) return null
  const selected = [...new Set(t.packages ?? MEMBERS.filter(c => !t.excluded.includes(c)))]
  const on = enabledFeatures(t, selected)
  const missing = selected.filter(c => (GATES[c] ?? []).some(f => !on.has(`${c}/${f}`)))
  if (missing.length === 0) return null
  // A plain name turns the feature on in every selected crate that has it, so one word fixes all.
  const features = [...new Set(missing.map(c => GATES[c]![0]!))]
  const tests = missing.length === 1 ? `${missing[0]}'s integration tests` : `the integration tests of ${listed(missing)}`
  const fix = quoteWords(withWords(t, ['--features', features.join(',')]))
  const single = t.packages !== null && selected.length === 1
  return `htui-dev: \`${quoteWords(t.words)}\` builds ${tests} without ${listed(features.map(f => `\`${f}\``))}, so they run 0 tests and pass — run: ${fix}${single ? '' : ' (or --all-features)'}`
}

/** G1 and G2 over one simple command: G2 first (its text is the more precise), then G1. */
export function cargoTest(words: Words): string | null {
  const t = parseCargoTest(words)
  if (t === null) return null
  return wrongFeature(t) ?? vacuousRun(t)
}

// --- G6 and G4's Bash half: git ---------------------------------------------------------------

/** git's own options before the subcommand that take a value as the next word (`-C dir`, `--git-dir x`). */
const GIT_GLOBAL_VALUE = new Set(['-C', '-c', '--git-dir', '--work-tree', '--namespace', '--exec-path', '--config-env'])

/** `git commit`'s short options whose value is the rest of the cluster or the next word. */
const COMMIT_SHORT_VALUE = new Set(['m', 'F', 'C', 'c', 't'])

/** `git commit`'s short options whose value, if any, is only the rest of the cluster (`-S<keyid>`, `-u<mode>`). */
const COMMIT_SHORT_OPTIONAL = new Set(['S', 'u'])

/** `git commit`'s long options that take the next word as their value when not given with `=`. */
const COMMIT_LONG_VALUE = new Set([
  '--message',
  '--file',
  '--reuse-message',
  '--reedit-message',
  '--template',
  '--author',
  '--date',
  '--fixup',
  '--squash',
  '--cleanup',
  '--trailer',
  '--pathspec-from-file',
])

/** The subcommand of a `git` command and where its arguments start, or null when `words` run no git. */
function gitSubcommand(words: Words): { sub: string; at: number } | null {
  if ((words[0] ?? '').split('/').pop() !== 'git') return null
  let at = 1
  while (at < words.length) {
    const w = words[at]!
    if (GIT_GLOBAL_VALUE.has(w)) at += 2
    else if (w.startsWith('-')) at++
    else return { sub: w, at: at + 1 }
  }
  return null
}

/** Whether a long option `w` is `full` or a prefix of it git would take (at least `min` characters). */
const abbreviates = (w: string, full: string, min: number): boolean => w.length >= min && full.startsWith(w)

/**
 * G6: `git commit` with `--no-verify` (or a prefix git takes) or `-n`, also inside a short-flag
 * cluster, unless a later `--verify` cancels it. The deny shows the commit without the flag.
 */
export function noVerify(words: Words): string | null {
  const git = gitSubcommand(words)
  if (git === null || git.sub !== 'commit') return null
  let skips = false
  const fixed: string[] = [...words.slice(0, git.at)]
  for (let i = git.at; i < words.length; i++) {
    const w = words[i]!
    if (w === '--') {
      fixed.push(...words.slice(i))
      break
    }
    if (w.startsWith('--')) {
      const name = w.split('=')[0]!
      if (abbreviates(name, '--no-verify', '--no-ve'.length) && !w.includes('=')) {
        skips = true
        continue
      }
      if (abbreviates(name, '--verify', '--veri'.length) && !w.includes('=')) skips = false
      fixed.push(w)
      if (COMMIT_LONG_VALUE.has(w) && i + 1 < words.length) fixed.push(words[++i]!)
      continue
    }
    if (!w.startsWith('-') || w === '-') {
      fixed.push(w)
      continue
    }
    // A short cluster: letters until one takes a value, which is the rest or the next word.
    let kept = '-'
    let takesNext = false
    for (let j = 1; j < w.length; j++) {
      const letter = w[j]!
      if (COMMIT_SHORT_VALUE.has(letter)) {
        kept += w.slice(j)
        takesNext = j === w.length - 1
        break
      }
      if (COMMIT_SHORT_OPTIONAL.has(letter)) {
        kept += w.slice(j)
        break
      }
      if (letter === 'n') skips = true
      else kept += letter
    }
    if (kept !== '-') fixed.push(kept)
    if (takesNext && i + 1 < words.length) fixed.push(words[++i]!)
  }
  if (!skips) return null
  return `htui-dev: git commit --no-verify skips the repo's commit hooks — commit without it: ${quoteWords(fixed)}`
}

/** git subcommands that always change the working tree, the index, refs or history (G4). */
const GIT_WRITES = new Set([
  'commit',
  'reset',
  'restore',
  'switch',
  'merge',
  'rebase',
  'cherry-pick',
  'revert',
  'am',
  'apply',
  'add',
  'rm',
  'mv',
  'pull',
])

/** `git checkout` options whose value is the next word. */
const CHECKOUT_VALUE = new Set(['-b', '-B', '--orphan'])

/** What `git checkout` writes, or null when it only switches branches: paths, `--force` or `--patch`. */
function checkoutWrite(args: Words): string | null {
  let positionals = 0
  for (let i = 0; i < args.length; i++) {
    const w = args[i]!
    if (w === '--') return 'git checkout <paths>'
    if (w === '--force') return 'git checkout --force'
    if (w === '--patch') return 'git checkout --patch'
    if (w.startsWith('--pathspec-from-file')) return 'git checkout <paths>'
    if (CHECKOUT_VALUE.has(w)) i++
    else if (/^-[A-Za-z]+$/.test(w)) {
      if (w.includes('f')) return 'git checkout --force'
      if (w.includes('p')) return 'git checkout --patch'
      if (/[bB]$/.test(w)) i++
    } else if (!w.startsWith('-')) {
      if (w === '.' || ++positionals >= 2) return 'git checkout <paths>'
    }
  }
  return null
}

/**
 * Whether a `git branch` or `git tag` command only lists: no write option (`writeLong`, or a short
 * cluster with a letter of `writeShort`), and a name given only to a list form (`--list`, `-l`, ...),
 * as a pattern, or as the value of a filter (`--contains <commit>`).
 */
function listsOnly(args: Words, writeLong: readonly string[], writeShort: string, listing: readonly string[]): boolean {
  const filters = ['--contains', '--no-contains', '--merged', '--no-merged', '--points-at', '--sort', '--format']
  let list = false
  let names = false
  for (let i = 0; i < args.length; i++) {
    const w = args[i]!
    const name = w.split('=')[0]!
    if (w === '--') {
      names ||= i + 1 < args.length
      break
    }
    if (writeLong.includes(name)) return false
    if (listing.includes(name)) list = true
    else if (filters.includes(w) && i + 1 < args.length && !args[i + 1]!.startsWith('-')) i++
    else if (/^-[A-Za-z]/.test(w) && !w.startsWith('--')) {
      const letters = w.slice(1).replace(/\d+$/, '')
      if ([...letters].some(l => writeShort.includes(l))) return false
      if ([...letters].some(l => listing.includes(`-${l}`))) list = true
    } else if (!w.startsWith('-')) names = true
  }
  return list || !names
}

/**
 * G4's Bash half: the git command that changes the working tree, the index, refs or history, as a
 * label for the refusal, or null. Reading stays allowed: status, diff, log, show, blame, fetch,
 * `stash list|show`, `checkout <branch>`, `clean -n`, and the list forms of `branch` and `tag`.
 */
export function gitWrite(words: Words): string | null {
  const git = gitSubcommand(words)
  if (git === null) return null
  const args = words.slice(git.at)
  if (GIT_WRITES.has(git.sub)) return `git ${git.sub}`
  switch (git.sub) {
    case 'stash':
      return args[0] === 'list' || args[0] === 'show' ? null : 'git stash'
    case 'checkout':
      return checkoutWrite(args)
    case 'clean':
      return args.some(w => w === '--dry-run' || /^-[A-Za-z]*n/.test(w)) ? null : 'git clean'
    case 'branch':
      return listsOnly(
        args,
        ['--delete', '--move', '--copy', '--force', '--set-upstream-to', '--unset-upstream', '--edit-description', '--track', '--no-track', '--create-reflog'],
        'dDmMcCfut',
        ['--list', '-l', '-a', '--all', '-r', '--remotes', '--show-current'],
      )
        ? null
        : 'git branch'
    case 'tag':
      return listsOnly(
        args,
        ['--annotate', '--sign', '--no-sign', '--local-user', '--force', '--delete', '--message', '--file', '--edit', '--create-reflog', '--trailer'],
        'asudmFfe',
        ['--list', '-l', '-n', '--verify', '-v'],
      )
        ? null
        : 'git tag'
    default:
      return null
  }
}

/**
 * A `git` word followed, past its global options, by a subcommand that always writes (`stash` but
 * its list and show). Only for a line the parser cannot follow; coarse on purpose.
 */
const COARSE_GIT_WRITE =
  /(?:^|[\s;&|()`'"])git(?:\s+(?:-C|-c|--git-dir|--work-tree|--namespace)\s+\S+|\s+-\S+)*\s+(commit|reset|restore|switch|merge|rebase|cherry-pick|revert|am|apply|add|rm|mv|pull|clean|stash(?!\s+(?:list|show)\b))(?![\w-])/

/**
 * G4's Bash half for a line the parser cannot follow (an unbalanced quote, `$((1<<2))` read as a
 * heredoc): the git write it seems to run, by a coarse pattern, or null. For a read-only agent only;
 * any other caller's unreadable line stays allowed.
 */
export function coarseGitWrite(command: string): string | null {
  const m = COARSE_GIT_WRITE.exec(command)
  return m === null ? null : `git ${m[1]}`
}

// --- G3: the forbidden literal --------------------------------------------------------------

/**
 * The name `crates/htui-agent/tests/extensibility.rs` forbids anywhere in `crates/**\/*.{rs,json}`
 * (case-sensitive), built so that this file never spells it.
 */
export const LITERAL = 'ze' + 'ta'

/**
 * The fields that carry the text a write puts into a file (Gortex: `new_string`, `new_source`,
 * `content`, `new_name`; Serena: `body`, `repl`, `content`, `new_name`); the old text is never read
 * (removing the literal is the fix).
 */
const NEW_TEXT = ['new_string', 'new_source', 'content', 'new_name', 'body', 'repl'] as const

/** Serena's one write tool that reaches many files: `relative_path` is a file, a folder, or empty for the whole project. */
const SERENA_MANY = /^mcp__(plugin_[a-z0-9_-]+_)?serena__replace_in_files$/

/** A path from a Gortex symbol id: what stands before `::` (`htui/crates/x.rs::Sym`). */
const idPath = (v: unknown): string | null => (typeof v === 'string' && v.includes('::') ? v.slice(0, v.indexOf('::')) : null)

/** The file a node of a tool's input names: `file_path`, `path`, `notebook_path`, `relative_path` (Serena), `target.file`, or a symbol id's file. */
function pathOf(node: Readonly<Record<string, unknown>>): string | null {
  for (const key of ['file_path', 'path', 'notebook_path', 'relative_path']) {
    if (typeof node[key] === 'string') return node[key] as string
  }
  const target = node.target
  if (target !== null && typeof target === 'object' && !Array.isArray(target)) {
    const t = target as Record<string, unknown>
    if (typeof t.file === 'string') return t.file
    const p = idPath(t.symbol) ?? idPath(t.id)
    if (p !== null) return p
  }
  return idPath(node.id)
}

/** Whether the literal scan reads `path`: a `.rs` or `.json` under a `crates/` folder, outside any `target/`, not the scan's own file. */
function scanned(path: string): boolean {
  const p = path.replace(/\\/g, '/')
  const at = p.lastIndexOf('crates/')
  if (at === -1 || (at > 0 && p[at - 1] !== '/')) return false
  const inCrates = p.slice(at)
  if (!/\.(rs|json)$/.test(inCrates)) return false
  if (inCrates.split('/').includes('target')) return false
  return inCrates !== 'crates/htui-agent/tests/extensibility.rs'
}

/** The first path, walking `node` with the nearest path passed down, whose new text holds the literal. */
function offender(node: unknown, inherited: string | null, depth: number): string | null {
  if (depth > 16 || node === null || typeof node !== 'object') return null
  if (Array.isArray(node)) {
    for (const item of node) {
      const hit = offender(item, inherited, depth + 1)
      if (hit !== null) return hit
    }
    return null
  }
  const record = node as Readonly<Record<string, unknown>>
  const path = pathOf(record) ?? inherited
  if (path !== null && scanned(path)) {
    for (const key of NEW_TEXT) {
      const v = record[key]
      if (typeof v === 'string' && v.includes(LITERAL)) return path
    }
  }
  for (const [key, v] of Object.entries(record)) {
    if (key === 'target') continue
    const hit = offender(v, path, depth + 1)
    if (hit !== null) return hit
  }
  return null
}

/** Whether a `replace_in_files` over `path` (a file, a folder, or empty for the whole project) can reach a file the scan reads. */
function reachesScanned(path: string): boolean {
  const parts = path.replace(/\\/g, '/').split('/').filter(p => p !== '' && p !== '.')
  if (parts.length === 0) return true
  if (/\.[A-Za-z0-9]+$/.test(parts[parts.length - 1]!)) return scanned(path)
  const at = parts.indexOf('crates')
  return at !== -1 && !parts.slice(at).includes('target')
}

/** A call that writes nothing although its tool can: Serena's `replace_in_files` as a dry run. */
export const writesNothing = (tool: string, input: Readonly<Record<string, unknown>>): boolean =>
  SERENA_MANY.test(tool) && input.dry_run === true

/** G3: an edit by `tool` that would write the literal into a file the scan reads; every matched tool's input is read the same way. */
export function forbiddenLiteral(tool: string, input: Readonly<Record<string, unknown>>): string | null {
  if (writesNothing(tool, input)) return null
  if (SERENA_MANY.test(tool)) {
    const where = typeof input.relative_path === 'string' ? input.relative_path : ''
    if (typeof input.repl !== 'string' || !input.repl.includes(LITERAL) || !reachesScanned(where)) return null
    const across = where.replace(/^\.?\/?$/, '') === '' ? 'the whole project' : where
    return `htui-dev: replacing across ${across} would write "${LITERAL}" into crates/**/*.{rs,json}, which crates/htui-agent/tests/extensibility.rs forbids — use another name, or narrow relative_path to files the scan does not read`
  }
  const path = offender(input, null, 0)
  if (path === null) return null
  return `htui-dev: ${path} would contain "${LITERAL}", which crates/htui-agent/tests/extensibility.rs forbids in crates/**/*.{rs,json} — use another name; only extensibility.rs may spell it`
}

// --- G4 and G5: subagents ---------------------------------------------------------------------

/** Agent types that review or design and never change the tree (Q2). A plugin's `<plugin>:<name>` counts by its name. */
const READ_ONLY_TYPES = new Set(['rust-reviewer', 'code-architect'])

/**
 * G4's marker (Q2): a prompt holding `READ-ONLY` (case-sensitive: a lowercase "read-only host tree"
 * is ordinary text) or "strictly no edits" (any case), or a read-only agent type. A tagged parent
 * is register.ts's to check: it lives in `$.state`.
 */
export function declaresReadOnly(spawn: { prompt: string; subagentType: string }): boolean {
  return (
    spawn.prompt.includes('READ-ONLY') ||
    /strictly no edits/i.test(spawn.prompt) ||
    READ_ONLY_TYPES.has(spawn.subagentType.split(':').pop()!)
  )
}

/** G4's deny for a file-writing tool called by the read-only agent `agentId`. */
export const readOnlyEdit = (tool: string, agentId: string): string =>
  `htui-dev: agent ${agentId} was started read-only and may not change files (${tool}) — report the change in your answer instead`

/** G4's deny for a git write (`gitWrite`'s label) run by the read-only agent `agentId`. */
export const readOnlyGit = (verb: string, agentId: string): string =>
  `htui-dev: agent ${agentId} was started read-only and may not run ${verb} — report what to change instead`

/** Whether a model name, alias or id, is a Fable model. */
export const isFable = (model: string): boolean => /fable/i.test(model)

/**
 * G5: a subagent asked to run on Fable. It reads the Agent tool's `model` parameter only: a model
 * left unset is resolved after the hook, from the agent definition or the parent (see register.ts).
 */
export function fableModel(model: string | undefined): string | null {
  if (model === undefined || !isFable(model)) return null
  return `htui-dev: subagents never run on Fable in this repo (model "${model}") — leave model unset so the agent inherits the session's model`
}

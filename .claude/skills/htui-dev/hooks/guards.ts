// The guards' decisions, pure: each reads plain words or a plain record and answers the deny
// reason (`htui-dev: <what> — <the fix>`), or null to let the call through. Nothing here calls
// `$`; register.ts wires them onto the events and catches what they throw.
import { quoteWords, type Words } from './shell'

// --- G1 and G2: cargo test -------------------------------------------------------------------

/**
 * The feature each member's integration tests are gated on (`#![cfg(feature = …)]` over
 * `crates/*\/tests`), from the `[features]` of `crates/*\/Cargo.toml`. A drift from the manifests
 * is TOOL-9's gate to catch.
 */
const GATED: Readonly<Record<string, string>> = {
  htui: 'testkit',
  'htui-agent': 'test-support',
  'htui-core': 'test-support',
  'htui-store': 'test-support',
}

/** Every member that has one of the two test features, gated tests or not: G2 checks the spelling. */
const TEST_FEATURE: Readonly<Record<string, string>> = {
  ...GATED,
  'htui-orch': 'test-support',
  'htui-worker': 'test-support',
}

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
 * The package a `--manifest-path` names: its folder's name, or null for the workspace root. A
 * member's folder counts only under `crates/` (or `../`), so `/r/htui/Cargo.toml` is the root.
 */
function manifestPackage(path: string): string | null {
  const parts = path.replace(/\\/g, '/').split('/').filter(p => p !== '' && p !== '.')
  const name = parts[parts.length - 2]
  if (name === undefined) return null
  const above = parts[parts.length - 3]
  if (!(MEMBERS as readonly string[]).includes(name)) return name
  return above === undefined || above === 'crates' || above === '..' ? name : null
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
    if (isOpt(w, '-p') || isOpt(w, '--package')) {
      const [v, n] = valueOf(i, w.startsWith('--') ? '--package' : '-p')
      if (v !== undefined) packages.push(v)
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
      if (v !== undefined) got.excluded.push(v)
      i += n
    } else if (w === '--all-features') {
      got.allFeatures = true
      i++
    } else if (isOpt(w, '--features') || isOpt(w, '-F')) {
      const name = w.startsWith('--') ? '--features' : '-F'
      if (w === name) {
        if (i + 1 < words.length) addFeatures(i + 1, '')
        i += 2
      } else {
        addFeatures(i, w.startsWith('--') ? '--features=' : '-F')
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

/** Whether `crate`'s gating feature `feature` is on: by `--all-features`, by name, or as `<crate>/<feature>`. */
const enables = (t: CargoTest, crate: string, feature: string): boolean =>
  t.allFeatures || t.features.includes(feature) || t.features.includes(`${crate}/${feature}`)

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
  const selected = (t.packages ?? [...MEMBERS]).filter(c => !t.excluded.includes(c))
  const missing = [...new Set(selected)].filter(c => GATED[c] !== undefined && !enables(t, c, GATED[c]!))
  if (missing.length === 0) return null
  const cmd = quoteWords(t.words)
  if (t.packages !== null && new Set(t.packages).size === 1) {
    const crate = missing[0]!
    const feature = GATED[crate]!
    return `htui-dev: \`${cmd}\` builds ${crate}'s integration tests without \`${feature}\`, so they run 0 tests and pass — run: ${quoteWords(withWords(t, ['--features', feature]))}`
  }
  return `htui-dev: \`${cmd}\` runs the feature-gated integration tests of ${missing.join(', ')} as 0 tests — run: ${quoteWords(withWords(t, ['--all-features']))}`
}

/** G1 and G2 over one simple command: G2 first (its text is the more precise), then G1. */
export function cargoTest(words: Words): string | null {
  const t = parseCargoTest(words)
  if (t === null) return null
  return wrongFeature(t) ?? vacuousRun(t)
}

// Reads a Bash line as the simple commands it runs, enough for the guards to see
// `cargo test …` and `git commit …` wherever they stand. It is no shell: what it
// cannot follow (an unbalanced quote, a heredoc with no end) answers null, and the
// guards let that call through (ANA-29 §6.3, "fail open").

/** One simple command's words: quotes removed, leading VAR=value and the wrappers env (and its -S string)/command/time/nice/timeout/nohup/exec stripped, redirections dropped. */
export type Words = readonly string[]

/**
 * What the tokenizer hands on: a word, a command separator, a redirection that does or does not
 * take the next word, or the text of a `$( … )` inside double quotes, whose commands run first.
 */
type Token =
  | { kind: 'word'; text: string }
  | { kind: 'sep' }
  | { kind: 'redir'; takesWord: boolean }
  | { kind: 'sub'; text: string }

/** A heredoc whose body starts at the next newline: its delimiter, and whether `<<-` lets the end line lead with tabs. */
type Heredoc = { delimiter: string; stripTabs: boolean }

/** Thrown inside the parser for a line it cannot follow; `simpleCommands` turns it into null. */
class Unfollowable extends Error {}

/** Characters that end an unquoted word. */
const BREAK = new Set([' ', '\t', '\n', ';', '&', '|', '(', ')', '<', '>', '`'])

/** Words that open or close a compound command; at the head of a command they are not the program. */
const KEYWORDS = new Set(['!', '{', '}', 'if', 'then', 'elif', 'else', 'fi', 'do', 'done', 'while', 'until'])

/** `NAME=value` or `NAME+=value`, a shell assignment. */
const ASSIGNMENT = /^[A-Za-z_][A-Za-z0-9_]*\+?=/

/** The shells whose `-c` string is followed. */
const SHELLS = new Set(['bash', 'sh', 'zsh', 'dash', 'ksh'])

/** The index of the `)` closing the `(` at `open`, counting parentheses only (for `$(( … ))`). Throws Unfollowable. */
function parenEnd(line: string, open: number): number {
  let depth = 0
  for (let i = open; i < line.length; i++) {
    if (line[i] === '(') depth++
    else if (line[i] === ')' && --depth === 0) return i
  }
  throw new Unfollowable('unbalanced $((')
}

/** Where a `$(` (or `$((`) at `dollar` ends: the index of its last `)`. Throws Unfollowable. */
const dollarParenEnd = (line: string, dollar: number): number =>
  line[dollar + 2] === '(' ? parenEnd(line, dollar + 1) : substitutionEnd(line, dollar + 2)

/** The index of the `"` closing a double-quoted string whose text starts at `from`. Throws Unfollowable. */
function doubleQuoteEnd(line: string, from: number): number {
  for (let i = from; i < line.length; ) {
    const d = line[i]!
    if (d === '"') return i
    if (d === '\\') i += 2
    else if (d === '$' && line[i + 1] === '(') i = dollarParenEnd(line, i) + 1
    else i++
  }
  throw new Unfollowable('unbalanced double quote')
}

/**
 * The index of the `)` closing a `$(` whose text starts at `from`. Inside, the shell starts afresh:
 * quotes, nested parentheses, comments and heredoc bodies are stepped over, so a `)` or `"` in them
 * closes nothing. Throws Unfollowable.
 */
function substitutionEnd(line: string, from: number): number {
  let depth = 0
  let wordStart = true
  const pending: Heredoc[] = []
  let i = from
  while (i < line.length) {
    const c = line[i]!
    if (c === '\\') {
      i += 2
      wordStart = false
      continue
    }
    if (c === "'") {
      const close = line.indexOf("'", i + 1)
      if (close === -1) throw new Unfollowable('unbalanced single quote')
      i = close + 1
      wordStart = false
      continue
    }
    if (c === '"') {
      i = doubleQuoteEnd(line, i + 1) + 1
      wordStart = false
      continue
    }
    if (c === '$' && line[i + 1] === '(') {
      i = dollarParenEnd(line, i) + 1
      wordStart = false
      continue
    }
    if (c === '#' && wordStart) {
      const eol = line.indexOf('\n', i)
      i = eol === -1 ? line.length : eol
      continue
    }
    if (line.startsWith('<<<', i)) {
      // A here-string: its word follows, no heredoc body.
      i += 3
      wordStart = true
      continue
    }
    if (c === '<' && line[i + 1] === '<') {
      i += 2
      const stripTabs = line[i] === '-'
      if (stripTabs) i++
      while (line[i] === ' ' || line[i] === '\t') i++
      let delimiter = ''
      while (i < line.length && !' \t\n;&|<>()'.includes(line[i]!)) {
        const q = line[i]!
        if (q === "'" || q === '"') {
          const close = line.indexOf(q, i + 1)
          if (close === -1) throw new Unfollowable('unbalanced quote in a heredoc delimiter')
          delimiter += line.slice(i + 1, close)
          i = close + 1
        } else if (q === '\\') {
          delimiter += line[i + 1] ?? ''
          i += 2
        } else {
          delimiter += q
          i++
        }
      }
      if (delimiter === '') throw new Unfollowable('heredoc without a delimiter')
      pending.push({ delimiter, stripTabs })
      wordStart = false
      continue
    }
    if (c === '\n') {
      i++
      for (const doc of pending.splice(0)) {
        for (;;) {
          if (i >= line.length) throw new Unfollowable('heredoc without an end')
          const eol = line.indexOf('\n', i)
          const end = eol === -1 ? line.length : eol
          const text = line.slice(i, end)
          i = end + 1
          if ((doc.stripTabs ? text.replace(/^\t+/, '') : text) === doc.delimiter) break
        }
      }
      wordStart = true
      continue
    }
    if (c === '(') depth++
    else if (c === ')') {
      if (depth === 0) return i
      depth--
    }
    wordStart = ' \t;&|()<>'.includes(c)
    i++
  }
  throw new Unfollowable('unbalanced $(')
}

/** Splits `line` into tokens, skipping comments and heredoc bodies. Throws Unfollowable. */
function tokenize(line: string): Token[] {
  const tokens: Token[] = []
  const heredocs: Heredoc[] = []
  /** Set once a redirection `<<` has read its operator: the next word is a heredoc delimiter. */
  let wantsDelimiter: Heredoc | null = null
  let word = ''
  let inWord = false
  let quoted = false
  let i = 0

  const endWord = () => {
    if (!inWord) return
    if (wantsDelimiter !== null) {
      heredocs.push({ ...wantsDelimiter, delimiter: word })
      wantsDelimiter = null
    } else {
      tokens.push({ kind: 'word', text: word })
    }
    word = ''
    inWord = false
    quoted = false
  }

  /** At a newline: skips the body of every heredoc opened on the line just ended. */
  const skipBodies = () => {
    for (const doc of heredocs.splice(0)) {
      for (;;) {
        if (i >= line.length) throw new Unfollowable('heredoc without an end')
        const eol = line.indexOf('\n', i)
        const end = eol === -1 ? line.length : eol
        const text = line.slice(i, end)
        i = end + 1
        if ((doc.stripTabs ? text.replace(/^\t+/, '') : text) === doc.delimiter) break
      }
    }
  }

  while (i < line.length) {
    const c = line[i]!
    if (c === '\\') {
      if (i + 1 >= line.length) throw new Unfollowable('trailing backslash')
      if (line[i + 1] === '\n') {
        i += 2
        continue
      }
      word += line[i + 1]
      inWord = true
      i += 2
      continue
    }
    if (c === "'") {
      const close = line.indexOf("'", i + 1)
      if (close === -1) throw new Unfollowable('unbalanced single quote')
      word += line.slice(i + 1, close)
      inWord = quoted = true
      i = close + 1
      continue
    }
    if (c === '"') {
      i++
      for (;;) {
        if (i >= line.length) throw new Unfollowable('unbalanced double quote')
        const d = line[i]!
        if (d === '"') break
        if (d === '\\' && i + 1 < line.length && '$`"\\\n'.includes(line[i + 1]!)) {
          if (line[i + 1] !== '\n') word += line[i + 1]
          i += 2
          continue
        }
        if (d === '$' && line[i + 1] === '(') {
          // A substitution: its text stays in the word, and its commands are handed on to run first.
          const end = dollarParenEnd(line, i)
          if (line[i + 2] !== '(') tokens.push({ kind: 'sub', text: line.slice(i + 2, end) })
          word += line.slice(i, end + 1)
          i = end + 1
          continue
        }
        word += d
        i++
      }
      inWord = quoted = true
      i++
      continue
    }
    if (c === '#' && !inWord) {
      const eol = line.indexOf('\n', i)
      i = eol === -1 ? line.length : eol
      continue
    }
    if (c === '$' && line[i + 1] === '(' && line[i + 2] === '(') {
      // `$(( … ))`: arithmetic, part of the word, no command (and its `<<` no heredoc).
      const end = parenEnd(line, i + 1)
      word += line.slice(i, end + 1)
      inWord = true
      i = end + 1
      continue
    }
    if (c === '$' && line[i + 1] === '(') {
      // `$(`: the substitution is a command of its own, opened by the `(` below.
      endWord()
      i++
      continue
    }
    if (!BREAK.has(c)) {
      word += c
      inWord = true
      i++
      continue
    }
    if (c === '(' && inWord && !quoted && /^[A-Za-z_][A-Za-z0-9_]*\+?=$/.test(word)) {
      // `arr=( … )`: an array assignment, one word, no subshell.
      const end = substitutionEnd(line, i + 1)
      word += line.slice(i, end + 1)
      i = end + 1
      continue
    }
    if (c === '<' || c === '>') {
      // A run of digits right before the operator names a file descriptor, not a word.
      if (inWord && !quoted && /^\d+$/.test(word)) {
        word = ''
        inWord = false
      }
      endWord()
      if (line.startsWith('<<<', i)) {
        tokens.push({ kind: 'redir', takesWord: true })
        i += 3
      } else if (line.startsWith('<<', i)) {
        const stripTabs = line[i + 2] === '-'
        i += stripTabs ? 3 : 2
        wantsDelimiter = { delimiter: '', stripTabs }
      } else {
        const two = line.slice(i, i + 2)
        const op = two === '>>' || two === '>&' || two === '<&' || two === '>|' || two === '<>' ? two : c
        i += op.length
        // `2>&1`, `>&-`: the operator names its target itself.
        const fd = op.endsWith('&') ? /^(\d+|-)/.exec(line.slice(i)) : null
        if (fd !== null) i += fd[0].length
        tokens.push({ kind: 'redir', takesWord: fd === null })
      }
      continue
    }
    if (c === '&' && line[i + 1] === '>') {
      endWord()
      i += line[i + 2] === '>' ? 3 : 2
      tokens.push({ kind: 'redir', takesWord: true })
      continue
    }
    endWord()
    if (wantsDelimiter !== null && c !== ' ' && c !== '\t') throw new Unfollowable('heredoc without a delimiter')
    if (c === ' ' || c === '\t') {
      i++
      continue
    }
    tokens.push({ kind: 'sep' })
    i += (c === '&' && line[i + 1] === '&') || (c === '|' && (line[i + 1] === '|' || line[i + 1] === '&')) ? 2 : 1
    if (c === '\n') skipBodies()
  }
  endWord()
  if (wantsDelimiter !== null) throw new Unfollowable('heredoc without a delimiter')
  if (heredocs.length > 0) skipBodies()
  return tokens
}

/**
 * The index past a wrapper's options from `at`: `withValue` and `longWithValue` name the options
 * whose value is the next word (`-n 10`); `-n10` and `--long=v` are one word, as is any other option.
 */
function skipOptions(words: string[], at: number, withValue: readonly string[], longWithValue: readonly string[]): number {
  while (at < words.length) {
    const w = words[at]!
    if (w === '--') return at + 1
    if (!w.startsWith('-') || w === '-') return at
    if (withValue.includes(w) || longWithValue.includes(w)) at += 2
    else at += 1
  }
  return at
}

/**
 * Where `env`'s options starting at `at` end, and the string of its `-S` (`--split-string`), whose
 * words stand in for the option and run as the command, or null without one.
 */
function envOptions(words: readonly string[], at: number): { at: number; split: string | null } {
  while (at < words.length) {
    const o = words[at]!
    if (o === '--') return { at: at + 1, split: null }
    if (!o.startsWith('-') || o === '-') return { at, split: null }
    if (o === '--split-string') return { at: at + 2, split: words[at + 1] ?? '' }
    if (o.startsWith('--split-string=')) return { at: at + 1, split: o.slice('--split-string='.length) }
    if (o === '--unset' || o === '--chdir') {
      at += 2
      continue
    }
    if (o.startsWith('--')) {
      at++
      continue
    }
    // A short cluster: `-u NAME` and `-C DIR` take a value, `-S STRING` the split string.
    let used = 1
    for (let j = 1; j < o.length; j++) {
      const letter = o[j]!
      if (letter !== 'u' && letter !== 'C' && letter !== 'S') continue
      const rest = o.slice(j + 1)
      if (letter === 'S') return rest !== '' ? { at: at + 1, split: rest } : { at: at + 2, split: words[at + 1] ?? '' }
      if (rest === '') used = 2
      break
    }
    at += used
  }
  return { at, split: null }
}

/** The words of `text` as the shell splits them: quotes removed, nothing run. Throws Unfollowable. */
const wordsOf = (text: string): string[] =>
  tokenize(text).flatMap(t => (t.kind === 'word' ? [t.text] : []))

/** The words with leading assignments, keywords and wrappers stripped, the program first. */
function unwrap(given: string[]): string[] {
  let words = given
  let at = 0
  for (;;) {
    const start = at
    while (at < words.length && (ASSIGNMENT.test(words[at]!) || KEYWORDS.has(words[at]!))) at++
    const w = words[at]
    if (w === 'env') {
      const env = envOptions(words, at + 1)
      if (env.split !== null) {
        // `env -S "cargo test"`: its string, split, is the command, followed by the words after it.
        words = [...wordsOf(env.split), ...words.slice(env.at)]
        at = 0
        continue
      }
      at = env.at
    } else if (w === 'exec') {
      at = skipOptions(words, at + 1, ['-a'], [])
    } else if (w === 'command' || w === 'nohup') {
      at = skipOptions(words, at + 1, [], [])
    } else if (w === 'time') {
      at = skipOptions(words, at + 1, ['-f', '-o'], ['--format', '--output'])
    } else if (w === 'nice') {
      at = skipOptions(words, at + 1, ['-n'], ['--adjustment'])
    } else if (w === 'timeout') {
      at = skipOptions(words, at + 1, ['-s', '-k'], ['--signal', '--kill-after'])
      if (at < words.length) at++ // the duration
    }
    if (at === start) return words.slice(at)
  }
}

/** The string a `bash|sh|zsh|dash|ksh -c` command runs, or null when the words are no such command. */
function dashC(words: readonly string[]): string | null {
  if (!SHELLS.has((words[0] ?? '').split('/').pop()!)) return null
  let hasC = false
  let at = 1
  while (at < words.length) {
    const w = words[at]!
    if (w === '-o' || w === '+o' || w === '-O' || w === '+O' || w === '--rcfile' || w === '--init-file') {
      at += 2
      continue
    }
    if (w === '--') {
      at++
      break
    }
    if (w.startsWith('--')) {
      at++
      continue
    }
    if (/^[-+][A-Za-z]+$/.test(w)) {
      if (w.startsWith('-') && w.includes('c')) hasC = true
      at++
      continue
    }
    break
  }
  return hasC && at < words.length ? words[at]! : null
}

/** The simple commands of `line`, `bash -c` strings followed. Throws Unfollowable. */
function commandsOf(line: string, depth: number): Words[] {
  if (depth > 8) throw new Unfollowable('bash -c nested too deep')
  const out: Words[] = []
  let words: string[] = []
  /** The commands of the substitutions in the command being read: they run before it. */
  let subs: Words[] = []
  let dropNext = false
  const flush = () => {
    out.push(...subs)
    subs = []
    const unwrapped = unwrap(words)
    words = []
    if (unwrapped.length === 0) return
    const inner = dashC(unwrapped)
    if (inner === null) out.push(unwrapped)
    else out.push(...commandsOf(inner, depth + 1))
  }
  for (const t of tokenize(line)) {
    if (t.kind === 'sub') {
      subs.push(...commandsOf(t.text, depth + 1))
    } else if (t.kind === 'sep') {
      dropNext = false
      flush()
    } else if (t.kind === 'redir') {
      dropNext = t.takesWord
    } else if (dropNext) {
      dropNext = false
    } else {
      words.push(t.text)
    }
  }
  flush()
  return out
}

/** The simple commands of a Bash line, recursing into `bash|sh|zsh|dash|ksh -c` (also `-lc`, `-ec`) and `$( … )` inside double quotes; null when the parser cannot follow it (unbalanced quote, trailing backslash, unterminated heredoc, nesting deep enough to overflow the stack). */
export function simpleCommands(command: string): Words[] | null {
  try {
    return commandsOf(command, 0)
  } catch (err) {
    if (err instanceof Unfollowable || err instanceof RangeError) return null
    throw err
  }
}

/** Words back to one shell line, single-quoting a word that needs it (for the corrected command in a deny). */
export function quoteWords(words: Words): string {
  return words.map(w => (/^[A-Za-z0-9_\-./=:,+@%^]+$/.test(w) ? w : `'${w.replace(/'/g, `'\\''`)}'`)).join(' ')
}

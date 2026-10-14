// htui-dev: guards for developing htui. Each refuses, with the fix in its reason, a kind of
// call that has cost this repo real work (docs/htui-dev.md):
//   G1  cargo test that builds feature-gated integration tests without their feature (0 tests, green)
//   G2  cargo test with the other crate's test feature name (testkit vs test-support)
// Every guard is a `{ deny }`, never a rewrite. A guard that throws lets the call through and
// says so on the status line (ANA-29 §6.3, "fail open"): the guards are a safety net, not a
// sandbox. The mod reads nothing outside the event; it runs no process and touches no file.
import type { EngineInterface, Register } from 'claude-code'
import { cargoTest } from './guards'
import { simpleCommands, type Words } from './shell'

/** Puts `text` on the plugin's status line; never throws (a `$` call in a re-entry rejects). */
function warn($: EngineInterface, text: string): void {
  try {
    $.ui.status(text)
  } catch {
    // Nothing is left to tell.
  }
}

/** Runs one guard; a throw is a status warning and no refusal (ANA-29 §6.3). */
function judge($: EngineInterface, guard: string, what: 'call' | 'spawn', decide: () => string | null): string | null {
  try {
    return decide()
  } catch {
    warn($, `htui-dev: ${guard} guard failed, ${what} allowed`)
    return null
  }
}

export const register: Register = on => {
  on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
    let cmds: Words[] | null = null
    try {
      cmds = simpleCommands(e.command)
    } catch {
      warn($, 'htui-dev: shell parser failed, call allowed')
    }
    // A line the parser cannot follow is allowed, with no status: it is no failure of the guard.
    if (cmds === null) return next(e)
    for (const words of cmds) {
      const reason = judge($, 'cargo test', 'call', () => cargoTest(words))
      if (reason !== null) return { deny: reason }
    }
    return next(e)
  }).catch(($, e, next) => {
    if (next.called) return next(e)
    warn($, 'htui-dev: tool.call guard failed, call allowed')
    return next(e)
  })
}

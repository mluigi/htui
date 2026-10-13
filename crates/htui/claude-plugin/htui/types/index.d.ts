/** One row of `htui waiting --json` as the mod reads it (docs/htui-cli.md, "waiting"). Other fields are ignored. */
export type HtuiRow = { reason: string; item_key: string; step: string | null; text: string; age_seconds: number }

/** The last poll: its rows, or the visible line for a poll that failed (then `rows` is empty). */
export type HtuiWaiting = {
  rows: readonly HtuiRow[]
  /** `workspace.name` as the top bar shows it, or null when the store has no workspace. */
  workspace: string | null
  /** `$.clock.now()` when the poll answered. */
  at: number
  error: string | null
}

declare module 'claude-code' {
  interface PluginState {
    htui: {
      /** null until the first poll answers. */
      waiting: HtuiWaiting | null
      /** D14's skew line, or null. */
      skew: string | null
      /** True once session.start ran and did not stand down (D15); the render hooks read it. */
      active: boolean
    }
  }
}

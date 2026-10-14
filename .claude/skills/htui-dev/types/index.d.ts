/** agentIds started read-only this session (G4); each is the agentId its loop's `tool.call` carries. */
export type HtuiDevReadOnly = readonly string[]

/** The `tool_use_id`s of the Workflow and Agent calls read-only agents made (G4): a spawn under one starts read-only. */
export type HtuiDevReadOnlyCalls = readonly string[]

/** The `finished` of the last gate result the status timer has seen (null: none had finished when it was seeded). */
export type HtuiDevGateSeen = { readonly finished: string | null }

declare module 'claude-code' {
  interface PluginState {
    'htui-dev': {
      /** Grows as read-only agents start; never shrinks within a session. */
      readOnly: HtuiDevReadOnly
      /** Grows as read-only agents call Workflow or Agent; never shrinks within a session. */
      readOnlyCalls: HtuiDevReadOnlyCalls
      /** Seeded when the status timer arms; a tick toasts when last.json's `finished` moves past it. */
      gateSeen: HtuiDevGateSeen | null
      /** session.start found the session fit for the status timer (interactive, not driven by htui); the tool.call hooks re-arm the timer from it after a reload. */
      ambient: boolean
    }
  }
}

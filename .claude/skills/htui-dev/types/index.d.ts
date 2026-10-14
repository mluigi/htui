/** agentIds started read-only this session (G4); each is the agentId its loop's `tool.call` carries. */
export type HtuiDevReadOnly = readonly string[]

/** The `tool_use_id`s of the Workflow and Agent calls read-only agents made (G4): a spawn under one starts read-only. */
export type HtuiDevReadOnlyCalls = readonly string[]

declare module 'claude-code' {
  interface PluginState {
    'htui-dev': {
      /** Grows as read-only agents start; never shrinks within a session. */
      readOnly: HtuiDevReadOnly
      /** Grows as read-only agents call Workflow or Agent; never shrinks within a session. */
      readOnlyCalls: HtuiDevReadOnlyCalls
    }
  }
}

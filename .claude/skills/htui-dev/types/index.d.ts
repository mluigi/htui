/** agentIds started read-only this session (G4); each is the agentId its loop's `tool.call` carries. */
export type HtuiDevReadOnly = readonly string[]

declare module 'claude-code' {
  interface PluginState {
    'htui-dev': {
      /** Grows as read-only agents start; never shrinks within a session. */
      readOnly: HtuiDevReadOnly
    }
  }
}

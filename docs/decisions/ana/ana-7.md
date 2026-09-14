# ANA-7 - Secret provider and scrubbing (concluded, 2026-09-14)

This item required analyzing the architectural approach to integrating the Infisical secret provider, authenticating the box, storing the credentials, and implementing a fail-closed transcript scrubber to prevent secrets from leaking into `htui`'s persistent store.

**Findings and Verdict:**
We analyzed the options and resolved the following architectural designs:

1. **Infisical SDK vs CLI**: We will use the `infisical-rust` SDK (or a native HTTP client) to communicate with Infisical. A CLI subprocess would introduce an external daemon dependency violating `R-NF-2` and complicate state management.
2. **Machine Identity Bootstrap**: Each `htui` box will authenticate to Infisical using Universal Auth (Machine Identity) by providing a `Client ID` and `Client Secret`.
3. **Keyring Usage**: `htui`'s Infisical Machine Identity credentials will be stored exclusively in the OS keyring using the `keyring` crate. The existing abstraction `htui-store::secret::Slot` will be extended to store `infisical-client-id` and `infisical-client-secret`.
4. **Scrub Mask Construction**: The `Scrubber` will use a two-pass mechanism. First, it will apply exact-match replacement (`[REDACTED: KEY_NAME]`) for all resolved secret values. Second, a fail-closed scan will check the resulting text against a static set of regular expressions covering known secret formats (e.g. AWS, Stripe, GitHub). If any pattern matches, the scrub operation fails and blocks persistence.

The full design and mapping is recorded in `docs/ANA-7.md`.

**Spawned/Unblocked Items:**
- **MOD-10**: Secret provider (now unblocked).

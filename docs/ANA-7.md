# ANA-7 - Secret provider and scrubbing

> **Scope note:** Analysis of the secret provider integration (`htui` acting as an Infisical client), machine identity bootstrapping, keyring usage for `htui`'s credentials, and the fail-closed scrubbing mechanism.
>
> **Requirements addressed:** `R-SEC-1..4`, `R-ID-7`, `R-NF-2`, `R-STO-1`.
>
> **Status (2026-09-14): concluded.** Implementation spawned as MOD-10.

---

## 1. Context and problem statement

`htui` must securely resolve project-specific secrets and inject them into the agent subprocess environment without exposing them to the agent's LLM context or `htui`'s persisted transcripts. This requires a `SecretProvider` (R-SEC-1) — initially Infisical — and a robust fail-closed scrubber (R-SEC-3) to prevent leaks. 
Additionally, `htui` needs to securely authenticate itself to the secret provider (and Postgres) using credentials stored in the OS keyring (R-STO-1).

This document settles the architecture for Infisical integration, authentication, credential storage, and transcript scrubbing.

---

## 2. Invariants

1. **Zero Secret Persistence:** Secrets resolved from the provider exist only in memory and in the environment block of spawned agent processes. They are never written to disk or the Postgres store.
2. **Fail-Closed Scrubbing (R-SEC-3):** Persistence is blocked if the scrubber detects known secret patterns after applying exact-match masks.
3. **No External Daemons (R-NF-2):** `htui` must not require external daemons beyond Postgres and the agents.
4. **Keyring Only (R-STO-1):** `htui`'s own credentials (Postgres DSN, Infisical Client Secret) reside strictly in the OS keyring.

---

## 3. Options and Verdicts

### 3.1 Infisical Integration: SDK vs CLI
**Need:** How should `htui` communicate with Infisical?

| Option | Verdict |
|---|---|
| Infisical CLI subprocess | Rejected. Violates `R-NF-2`'s spirit by requiring an external CLI tool installed on every box. Managing authentication state across subprocesses is brittle. |
| Infisical Rust SDK / Native HTTP Client | **Adopted.** `htui` will use the `infisical-rust` SDK (or a native HTTP client against the Infisical API). This keeps dependencies in-process, allows precise error handling (R-SEC-4), and avoids requiring users to install the Infisical CLI on every box. |

### 3.2 Machine Identity Bootstrap
**Need:** How does a box authenticate with Infisical?

| Option | Verdict |
|---|---|
| User OAuth/Token | Rejected. Tokens expire and require interactive login flows, interrupting automated runs (R-ORCH-6). |
| Universal Auth (Machine Identity) | **Adopted.** Each `htui` box uses Infisical Universal Auth (Machine Identity). The user provisions a Machine Identity in Infisical for the box and supplies the `Client ID` and `Client Secret` to `htui` via a one-time TUI configuration in the Settings tab. |

### 3.3 Keyring Usage for htui's Credentials
**Need:** Where are the Postgres DSN (R-STO-1) and Infisical Machine Identity stored?

| Option | Verdict |
|---|---|
| Encrypted local file | Rejected. Requires a master password, complicating unattended startup. |
| OS Keyring (`keyring` crate) | **Adopted.** The Rust `keyring` crate is already in use (`htui-store::secret::Slot`). We extend it to store `infisical-client-id` and `infisical-client-secret` under the `htui` service. Values are read directly from the keyring at startup. |

### 3.4 Scrub Mask Construction and Fail-Closed Check
**Need:** How to guarantee secrets do not leak into transcripts (R-SEC-3, R-ID-7).

| Component | Design |
|---|---|
| Exact-Match Masking | Collect all resolved secret values (length >= 6 to avoid false positives). Replace every occurrence with `[REDACTED: KEY_NAME]`. |
| Pattern Rule Set | A static list of regexes for known secret formats (e.g. AWS `AKIA...`, GitHub `ghp_...`, Stripe `sk_live_...`). |
| Fail-Closed Check | **After** exact-match masking, scan the transcript string with the pattern rule set. If a pattern matches, the scrub fails. The row is dropped, the step is marked `failed` with `terminal_reason: "scrubber_fail_closed"`, and the run halts. |

---

## 4. Phasing (MOD spawn plan)

This analysis gates **MOD-10 - Secret provider**.
- Extend `htui-store::secret::Slot` to manage `infisical-client-id` and `infisical-client-secret`.
- Implement `SecretProvider` trait with an Infisical native client using Universal Auth.
- Implement the two-pass `Scrubber` (exact match + regex pattern validation) returning `Result<String, ScrubError>`.
- Wire the provider to the agent subprocess environment spawner.

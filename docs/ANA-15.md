# ANA-15 - Bugsink integration via Sentry crate

> **Scope note:** Research on whether the self-hosted Bugsink error tracking platform can be integrated using the standard `sentry` Rust crate and its ecosystem of support crates (`sentry-anyhow`, `sentry-tracing`, etc.), given Bugsink's stated Sentry SDK compatibility.
>
> **Requirements addressed:** Error tracking telemetry (implied).
>
> **Status (2026-09-14): concluded.** Implementation tracked as MOD-29 in `HANDOFF.md`. No migration file needed.

---

## 1. Context and problem statement

`htui` requires a mechanism for error telemetry. Bugsink is an open-source, self-hostable error-tracking platform positioned as a lightweight, privacy-focused alternative to Sentry. 

The question is whether `htui` can leverage the official `sentry` Rust crate ecosystem to report errors to Bugsink, or if a custom client is required. Specifically, whether support crates like `sentry-anyhow` function correctly when the destination is Bugsink rather than Sentry SaaS.

## 2. Invariants

1. **No custom error reporting clients.** If an off-the-shelf SDK can reliably transmit errors to the telemetry backend, `htui` must use it rather than maintaining a bespoke HTTP client.
2. **Client-side error wrapping must remain idiomatic.** The use of `anyhow` for error handling must not be compromised by the telemetry integration.

## 3. Surface as read

- Bugsink implements the standard Sentry event ingestion API.
- The `sentry` Rust crate (the official SDK) allows arbitrary DSN URLs, routing standard Sentry JSON payloads to any compliant endpoint.
- Support crates (`sentry-anyhow`, `sentry-tracing`, `sentry-log`, `sentry-panic`) operate entirely on the client side. They bridge Rust-specific types (e.g., `anyhow::Error`, tracing spans, panic payloads) into the standard Sentry protocol `Event` structs before passing them to the Sentry Hub for transmission.

## 4. Settled questions

### 4.1 Q1 - Can Bugsink be implemented via the `sentry` crate?

**Verdict:** Yes.

**Reasoning:** Bugsink acts as a drop-in replacement for a Sentry server. Pointing the `sentry` crate's DSN (Data Source Name) to a Bugsink instance is the fully supported, documented integration path for Bugsink. The SDK serializes errors identically, and Bugsink ingests the Sentry-compatible payloads.

### 4.2 Q2 - Are support crates like `sentry-anyhow` compatible?

**Verdict:** Yes.

**Reasoning:** Because the support crates operate entirely locally—translating `anyhow::Error` metadata into Sentry breadcrumbs and exception payloads—the network transmission remains identical to a standard Sentry event. Bugsink receives this standard payload and processes it without issue. 

## 5. Phasing and Next Steps

The research confirms feasibility with zero custom networking code required.

- **Spawn MOD-29:** Add the `sentry` and `sentry-anyhow` dependencies, configure the DSN to point to the Bugsink instance, and wire the initialization into the application startup.

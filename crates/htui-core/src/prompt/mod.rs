//! The prompt assembler: everything pure (ANA-5 §4.8 `:1458-1477`).
//!
//! ANA-5 §4.8 split the prompt builder along the dependency test ANA-4 §8 applied to the JSON-RPC
//! SDK. The pure half lives here, in `htui-core`, because that is the only crate both MOD-2 and
//! MOD-9 reach without a new edge: MOD-9's template editor lives in `crates/htui`, which depends on
//! `htui-core` and `htui-store` and on neither `htui-agent` nor `htui-orch`, while MOD-4 needs the
//! same one assembler for the judge and handoff prompts. The I/O half — the repository walk and the
//! per-file read behind [`crate::prompt`]'s excerpt section — is deliberately **not** here: this
//! crate contains no `std::fs` at all, and `htui_agent::excerpt` owns that side.
//!
//! One assembler, three roles (ANA-5 §7): a phase step's prompt, the judge's, and the handoff's.
//! `assemble()` is pure by contract (ANA-5 invariant 2 `:1975-1976`) — no store handle, no I/O, no
//! clock, and no map iteration order reaches the rendered bytes — which is what makes a prompt
//! digest reproducible on any box.
//!
//! Milestone 9 builds this module in pieces; T59 lands the two leaves that depend on nothing:
//! [`template`], the `{{name}}` scanner and its closed per-role placeholder sets, and [`estimate`],
//! the `chars-v2` token estimator every budget decision is measured with.

pub mod estimate;
pub mod template;

pub use estimate::TokenEstimator;
pub use template::{ParsedTemplate, Placeholder, Span, TemplateError, TemplateRole, parse};

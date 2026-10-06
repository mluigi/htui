//! `htui-secrets`: htui's secret providers (MOD-10 M2). Today one: [`InfisicalProvider`], an
//! in-process Infisical client behind `htui_core::secret::SecretProvider`.
//!
//! It never reads the keyring. The caller reads the `MachineIdentity` and base URL from
//! `htui-store::secret` and hands them over (M3 composes the two in the process that runs the run).
//! Share **one** provider per process and identity: the login latch (D5) is per instance.
#![warn(missing_docs)]

mod infisical;
mod wire;

pub use infisical::{
    DEFAULT_CONNECT_TIMEOUT, DEFAULT_LOGIN_COOL_DOWN, DEFAULT_TIMEOUT, InfisicalConfig,
    InfisicalProvider, normalise_base_url,
};

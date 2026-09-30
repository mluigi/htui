//! `htui worker` (MOD-41 plan D14, D15): the headless connect, then `htui_worker::worker::run`.

use std::path::Path;

use htui_store::pg::{CONNECT_TIMEOUT, PoolSize};
use htui_store::{PgStore, connect, identity};

use crate::concepts;

/// How `htui worker` ends (plan D14): `main` maps it to the exit code.
#[derive(Debug)]
pub enum WorkerExit {
    /// Exit 2: nothing ran (no DSN, connect or schema refused, signals not installable).
    Refused(String),
    /// Exit 1: the worker ran and failed.
    Failed(String),
}

impl WorkerExit {
    /// 2 for a refusal, 1 for a failure.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Refused(_) => 2,
            Self::Failed(_) => 1,
        }
    }
}

impl core::fmt::Display for WorkerExit {
    /// The sentence, and nothing else.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Refused(sentence) | Self::Failed(sentence) => f.write_str(sentence),
        }
    }
}

impl std::error::Error for WorkerExit {}

/// The headless connect and the registration write-back (plan D14): every refusal before any
/// write, with `concepts::headless_refusal`'s sentences. `pub` for `worker_pg.rs` case 4.
///
/// # Errors
///
/// [`WorkerExit::Refused`].
pub async fn connect(dsn: &str, root: &Path, pool: PoolSize) -> Result<PgStore, WorkerExit> {
    let presented =
        identity::load_or_mint(root).map_err(|err| WorkerExit::Refused(err.to_string()))?;
    let pg = PgStore::connect_headless(dsn, &presented, CONNECT_TIMEOUT, pool)
        .await
        .map_err(|err| WorkerExit::Refused(format!("{:#}", concepts::headless_refusal(err))))?;
    connect::persist_registration(root, &presented, &pg)
        .map_err(|err| WorkerExit::Refused(err.to_string()))?;
    Ok(pg)
}

#[cfg(test)]
mod tests {
    use super::WorkerExit;

    /// Plan D14: a startup refusal exits 2, a failure after start exits 1.
    #[test]
    fn worker_exit_codes() {
        assert_eq!(WorkerExit::Refused("no DSN".to_owned()).code(), 2);
        assert_eq!(WorkerExit::Failed("lost".to_owned()).code(), 1);
    }
}

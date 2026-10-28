//! `htui --set-qdrant-url`, `--clear-qdrant-url`, `--set-qdrant-key` and `--clear-qdrant-key`
//! (MOD-158 D8): each acts, prints one line on stderr and exits, before any terminal work.
//!
//! The URL is one database-wide setting, the `app_setting` row `qdrant_url` (D1), so the URL flags
//! connect headless with the keyring's DSN, as `--index-items` does, and write through D1's
//! compare-and-set, read-then-write. The URL is no secret, so it may be an argument; it is checked
//! by `QdrantUrl::parse` (D2) before the keyring is read, and a refusal never echoes it. The API
//! key is this box's: one line of stdin into the OS keyring, as `--set-dsn`, never an argument.
//!
//! Every ending is a [`StateExit`] (blueprint P-11): Refused 2, Offline 3, Failed 1, which `main`
//! already maps and keeps 2 and 3 out of crash reports.

use chrono::{DateTime, Utc};
use htui_core::qdrant::{QdrantUrl, StoredQdrantUrl};
use htui_core::store::{CasOutcome, StoreError};
use htui_store::pg::{CONNECT_TIMEOUT, PoolSize};
use htui_store::secret::QdrantEnv;
use htui_store::{PgStore, identity, secret};

use crate::state_cmd::{self, ErrorCode, Failure, StateExit, StoreState};

/// What `--set-qdrant-key` says on a terminal before it reads the line.
pub const KEY_PROMPT: &str = "paste the Qdrant API key and press Enter (it will be visible):";

/// `--set-qdrant-key` with a blank line or no line at all.
pub const NO_KEY_LINE: &str = "no Qdrant API key on stdin; nothing was stored";

/// A set whose compare-and-set missed: the row moved between the read and the write.
pub const SET_STALE: &str = "the Qdrant URL changed while it was being set; nothing was written \
                             — run the command again";

/// A clear whose compare-and-set missed.
pub const CLEAR_STALE: &str = "the Qdrant URL changed while it was being cleared; nothing was \
                               removed — run the command again";

/// The URL flags' refusal when the OS keyring cannot be read for the DSN, before the error.
pub const KEYRING_UNREADABLE: &str = "the OS keyring could not be read";

/// `--clear-qdrant-url` with no row: not a failure.
pub const NOTHING_TO_CLEAR: &str = "no Qdrant URL is set; nothing was cleared";

/// `--set-qdrant-url URL`: refused by D2 before the keyring is read, then the headless connect,
/// then [`set_url_on`]. With `HTUI_QDRANT_URL` set in this process, the line says it overrides
/// the setting here. Prints the line on stderr.
///
/// # Errors
/// [`StateExit`]: 2 for a refused URL, no DSN, the keyring, `box.toml`, the schema or a stale
/// write; 3 when Postgres does not answer; 1 for any other store failure.
pub async fn set_url(text: &str) -> Result<(), StateExit> {
    parse(text)?;
    let pg = connect().await?;
    // Read only once the write can happen, so no refusal reads the process's environment.
    let env = QdrantEnv::from_process();
    let ended = set_url_on(&pg, text, env.url.as_deref()).await;
    pg.close().await;
    eprintln!("{}", ended?);
    Ok(())
}

/// `--clear-qdrant-url`: the headless connect, then [`clear_url_on`]. Prints the line on stderr.
///
/// # Errors
/// As [`set_url`], without the URL refusal.
pub async fn clear_url() -> Result<(), StateExit> {
    let pg = connect().await?;
    let ended = clear_url_on(&pg).await;
    pg.close().await;
    eprintln!("{}", ended?);
    Ok(())
}

/// `--set-qdrant-key`: one line of `reader`, trimmed, into this box's OS keyring. The rest of
/// `reader` is left unread. Answers the line to print.
///
/// # Errors
/// [`StateExit::Refused`] for a blank line or a keyring that refuses the write;
/// [`StateExit::Failed`] when `reader` fails. Never the key.
pub fn set_key(reader: &mut dyn std::io::BufRead) -> Result<String, StateExit> {
    let line = secret::read_dsn_line(reader).map_err(|err| {
        StateExit::Failed(format!("cannot read the Qdrant API key from stdin: {err}"))
    })?;
    let Some(key) = line else {
        return Err(StateExit::Refused(NO_KEY_LINE.to_owned()));
    };
    secret::set_qdrant_api_key(&key).map_err(|err| {
        StateExit::Refused(format!(
            "the Qdrant API key could not be stored in the OS keyring: {err}"
        ))
    })?;
    Ok("Qdrant API key stored in this box's OS keyring.".to_owned())
}

/// `--clear-qdrant-key`: removes this box's key from the OS keyring; a missing one is no failure.
/// Answers the line to print.
///
/// # Errors
/// [`StateExit::Refused`] when the keyring refuses the delete.
pub fn clear_key() -> Result<String, StateExit> {
    secret::clear_qdrant_api_key().map_err(|err| {
        StateExit::Refused(format!(
            "the Qdrant API key could not be removed from the OS keyring: {err}"
        ))
    })?;
    Ok("Qdrant API key removed from this box's OS keyring.".to_owned())
}

/// [`set_url`]'s core over a connected store: D2, then the row's token, then
/// [`set_url_expecting`] with it. `env_url` is `HTUI_QDRANT_URL` as the caller read it (a test's
/// literal, never the process's: blueprint P-13). Answers the line to print.
///
/// # Errors
/// As [`set_url`].
pub async fn set_url_on(
    pg: &PgStore,
    text: &str,
    env_url: Option<&str>,
) -> Result<String, StateExit> {
    let url = parse(text)?;
    let expected = stored(pg).await?.map(|row| row.token);
    set_url_expecting(pg, &url, expected).await?;
    let set = format!("Qdrant URL set for every box: {url}.");
    Ok(match env_url {
        Some(_) => format!(
            "{set} {} overrides it in processes that have it.",
            htui_core::secret::QDRANT_URL_ENV
        ),
        None => set,
    })
}

/// [`clear_url`]'s core over a connected store: the row's token, then [`clear_url_expecting`].
/// No row is [`NOTHING_TO_CLEAR`], not a failure. A malformed row is cleared like any other
/// (blueprint P-3). Answers the line to print.
///
/// # Errors
/// As [`clear_url`].
pub async fn clear_url_on(pg: &PgStore) -> Result<String, StateExit> {
    let Some(row) = stored(pg).await? else {
        return Ok(NOTHING_TO_CLEAR.to_owned());
    };
    clear_url_expecting(pg, row.token).await?;
    Ok("Qdrant URL cleared for every box.".to_owned())
}

/// D1's compare-and-set write of `url`, expecting the row at `expected` (`None`: no row). A miss
/// is [`SET_STALE`], and nothing is written.
pub(crate) async fn set_url_expecting(
    pg: &PgStore,
    url: &QdrantUrl,
    expected: Option<DateTime<Utc>>,
) -> Result<(), StateExit> {
    match pg
        .set_qdrant_url(url, expected)
        .await
        .map_err(|err| store_exit(&err, "written"))?
    {
        CasOutcome::Applied(_) => Ok(()),
        CasOutcome::Stale(_) => Err(StateExit::Refused(SET_STALE.to_owned())),
    }
}

/// D1's compare-and-set delete of the row at `expected`. A miss is [`CLEAR_STALE`], and nothing
/// is removed.
pub(crate) async fn clear_url_expecting(
    pg: &PgStore,
    expected: DateTime<Utc>,
) -> Result<(), StateExit> {
    match pg
        .clear_qdrant_url(expected)
        .await
        .map_err(|err| store_exit(&err, "cleared"))?
    {
        CasOutcome::Applied(_) => Ok(()),
        CasOutcome::Stale(_) => Err(StateExit::Refused(CLEAR_STALE.to_owned())),
    }
}

/// D2, as `--set-qdrant-url`'s refusal. The sentence is `QdrantUrlError`'s own (blueprint P-20),
/// which never carries the text.
fn parse(text: &str) -> Result<QdrantUrl, StateExit> {
    QdrantUrl::parse(text)
        .map_err(|why| StateExit::Refused(format!("not setting the Qdrant URL: {why}")))
}

/// The row as it is, malformed or not.
async fn stored(pg: &PgStore) -> Result<Option<StoredQdrantUrl>, StateExit> {
    pg.qdrant_url()
        .await
        .map_err(|err| store_exit(&err, "read"))
}

/// The keyring's DSN, `box.toml` (minted on a first run, as `--index-items` does), then the
/// headless connect: a pending schema or a build below the database's target is refused, never
/// migrated. Every refusal is `state_cmd`'s sentence, but an unreadable keyring says so first
/// ([`KEYRING_UNREADABLE`]).
async fn connect() -> Result<PgStore, StateExit> {
    let dsn = state_cmd::read_dsn().map_err(dsn_exit)?;
    let identity = identity::config_root()
        .and_then(|root| identity::load_or_mint(&root))
        .map_err(|err| StateExit::Refused(err.to_string()))?;
    PgStore::connect_headless(&dsn, &identity, CONNECT_TIMEOUT, PoolSize::TUI)
        .await
        .map_err(|err| failure_exit(Failure::from_connect(err)))
}

/// [`state_cmd::read_dsn`]'s refusal as this command's ending: `NO_DSN` as it is, the keyring's
/// own error after [`KEYRING_UNREADABLE`] (blueprint T5.2).
fn dsn_exit(failure: Failure) -> StateExit {
    match failure.error.code {
        ErrorCode::KeyringUnreadable => {
            StateExit::Refused(format!("{KEYRING_UNREADABLE}: {}", failure.error.message))
        }
        _ => failure_exit(failure),
    }
}

/// A state verb's [`Failure`] as this command's ending: offline 3, refused 2, anything else 1.
fn failure_exit(failure: Failure) -> StateExit {
    let message = failure.error.message;
    match failure.store {
        StoreState::Offline => StateExit::Offline(message),
        StoreState::Refused => StateExit::Refused(message),
        StoreState::Online | StoreState::Error => StateExit::Failed(message),
    }
}

/// A store call after the connect: `Unreachable` is offline (3), anything else a failure (1).
fn store_exit(err: &StoreError, done: &str) -> StateExit {
    match err {
        StoreError::Unreachable(_) => StateExit::Offline(format!("cannot reach Postgres: {err}")),
        _ => StateExit::Failed(format!("the Qdrant URL setting could not be {done}: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CLEAR_STALE, SET_STALE, clear_url_expecting, failure_exit, set_url_expecting, set_url_on,
        store_exit,
    };
    use crate::state_cmd::{Failure, StateExit};
    use htui_core::qdrant::QdrantUrl;
    use htui_core::store::StoreError;
    use htui_store::{HeadlessError, testkit};

    fn url(text: &str) -> QdrantUrl {
        QdrantUrl::parse(text).expect("a Qdrant URL")
    }

    /// The row's value as text, or `None`.
    async fn row(db: &testkit::TestDb) -> Option<String> {
        db.store
            .qdrant_url()
            .await
            .expect("read")
            .map(|row| row.usable().expect("usable").to_string())
    }

    /// Blueprint T5.3: another writer moved the row between the read and the write; the write
    /// presents the spent token, is refused as stale, and writes nothing.
    #[tokio::test]
    async fn a_set_with_a_spent_token_is_stale_and_writes_nothing() {
        let Some(db) = testkit::demo_db().await else {
            return;
        };
        set_url_on(&db.store, "http://qdrant-a.lan:6334", None)
            .await
            .expect("set");
        let spent = db
            .store
            .qdrant_url()
            .await
            .expect("read")
            .expect("row")
            .token;
        set_url_on(&db.store, "http://qdrant-b.lan:6334", None)
            .await
            .expect("a concurrent set");
        let ended = set_url_expecting(&db.store, &url("http://qdrant-c.lan"), Some(spent)).await;
        assert!(
            matches!(&ended, Err(StateExit::Refused(sentence)) if sentence == SET_STALE),
            "{ended:?}"
        );
        assert_eq!(ended.expect_err("stale").code(), 2);
        assert_eq!(
            SET_STALE,
            "the Qdrant URL changed while it was being set; nothing was written — run the command \
             again"
        );
        // Expecting no row while one exists is stale too.
        let ended = set_url_expecting(&db.store, &url("http://qdrant-c.lan"), None).await;
        assert!(
            matches!(&ended, Err(StateExit::Refused(sentence)) if sentence == SET_STALE),
            "{ended:?}"
        );
        assert_eq!(row(&db).await.as_deref(), Some("http://qdrant-b.lan:6334"));
        db.drop_db().await;
    }

    /// A clear presenting a spent token is stale and removes nothing; a token for a row that is
    /// gone is stale too.
    #[tokio::test]
    async fn a_clear_with_a_spent_token_is_stale_and_removes_nothing() {
        let Some(db) = testkit::demo_db().await else {
            return;
        };
        set_url_on(&db.store, "http://qdrant-a.lan:6334", None)
            .await
            .expect("set");
        let spent = db
            .store
            .qdrant_url()
            .await
            .expect("read")
            .expect("row")
            .token;
        set_url_on(&db.store, "http://qdrant-b.lan:6334", None)
            .await
            .expect("a concurrent set");
        let ended = clear_url_expecting(&db.store, spent).await;
        assert!(
            matches!(&ended, Err(StateExit::Refused(sentence)) if sentence == CLEAR_STALE),
            "{ended:?}"
        );
        assert_eq!(row(&db).await.as_deref(), Some("http://qdrant-b.lan:6334"));
        let current = db
            .store
            .qdrant_url()
            .await
            .expect("read")
            .expect("row")
            .token;
        clear_url_expecting(&db.store, current)
            .await
            .expect("cleared");
        let ended = clear_url_expecting(&db.store, current).await;
        assert!(
            matches!(&ended, Err(StateExit::Refused(sentence)) if sentence == CLEAR_STALE),
            "{ended:?}"
        );
        db.drop_db().await;
    }

    /// The connect's refusals keep `state_cmd`'s sentences and map to its exits.
    #[test]
    fn a_connect_failure_ends_as_the_state_verbs_do() {
        let offline = failure_exit(Failure::from_connect(HeadlessError::Store(
            StoreError::Unreachable("connection refused".into()),
        )));
        assert_eq!(offline.code(), 3);
        assert!(
            offline.to_string().starts_with("cannot reach Postgres"),
            "{offline}"
        );
        let pending = failure_exit(Failure::from_connect(HeadlessError::MigrationsPending(2)));
        assert_eq!(pending.code(), 2);
        assert_eq!(
            pending.to_string(),
            "2 schema migration(s) are pending; start `htui` once to apply them"
        );
        let refused = failure_exit(Failure::from_connect(HeadlessError::Store(
            StoreError::Backend("28P01: password failed".into()),
        )));
        assert_eq!(refused.code(), 2);
    }

    /// After the connect: an unreachable server is offline (3), anything else a failure (1).
    #[test]
    fn a_store_failure_after_the_connect_is_offline_or_failed() {
        let offline = store_exit(&StoreError::Unreachable("gone".into()), "written");
        assert_eq!(offline.code(), 3);
        assert!(offline.to_string().starts_with("cannot reach Postgres: "));
        let failed = store_exit(&StoreError::Backend("disk full".into()), "written");
        assert_eq!(failed.code(), 1);
        assert!(
            failed
                .to_string()
                .starts_with("the Qdrant URL setting could not be written: "),
            "{failed}"
        );
    }
}

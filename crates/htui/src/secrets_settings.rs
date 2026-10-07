//! The keyring rows of `Settings > Secrets`, and the redacting types its writes travel in (MOD-10
//! milestone 4, D2–D5, D9, D10).
//!
//! [`snapshot`] is the read; [`serve`] answers the read and the four keyring writes for
//! `try_serve`. None of the five needs loop state, so the loop's `other => try_serve` serves them,
//! and the test harness and `--demo` get the same answer (blueprint B.4).
//!
//! Nothing here can hand a value to the UI. The URL row carries the stored URL's **normalised**
//! form, which has no user info, query or fragment; a stored text that does not normalise is
//! reported by normalisation's own sentence, never echoed. The identity row is a state: the
//! identity read for it is dropped, wiped, inside the blocking task that read it.
//!
//! [`Redacted`] and [`IdentityEntry`] are the request field types (blueprint A-9): `StoreRequest`
//! derives `Debug` and `Clone`, so a secret on it is cloned zeroizing and prints `<redacted>`.

use chrono::{DateTime, Utc};
use htui_core::model::ProjectId;
use htui_core::secret::{MachineIdentity, ProviderHealth, SecretError};
use htui_core::store::{Result, StoreError};
use htui_store::{Backend, secret};
use zeroize::Zeroizing;

use crate::store_worker::{StoreReply, StoreRequest};

/// Every keyring request this module answers, in `StoreRequest` order: the read, then the four
/// writes. [`StoreRequest::name`]'s arms and the section's `Failed` match both read from here.
pub const REQUEST_NAMES: [&str; 5] = [
    "secrets_info",
    "set_infisical_url",
    "clear_infisical_url",
    "set_machine_identity",
    "clear_machine_identity",
];

/// The read's name: a refused read leaves the section with no snapshot, a refused write keeps
/// the rows it had.
pub const READ_NAME: &str = REQUEST_NAMES[0];

/// [`StoreRequest::CheckSecretProvider`]'s name (D5).
pub const CHECK_SECRET_PROVIDER: &str = "check_secret_provider";

/// [`StoreRequest::CheckSecretScope`]'s name (D8).
pub const CHECK_SECRET_SCOPE: &str = "check_secret_scope";

/// [`StoreRequest::SetProjectSecretScope`]'s name (D6). Not one of
/// [`hierarchy::REQUEST_NAMES`](crate::hierarchy::REQUEST_NAMES) (blueprint A-11): its refusals
/// are the Secrets section's, not the Hierarchy section's.
pub const SET_PROJECT_SECRET_SCOPE: &str = "set_project_secret_scope";

/// What a keyring write is refused with on [`Backend::Memory`] (D10, blueprint A-8).
pub const DEMO_SESSION: &str = "a demo session has no keyring to change";

/// What an identity with a blank half is refused with: a blank half reads as absent, so storing
/// it would store a half identity (D4).
pub const IDENTITY_INCOMPLETE: &str =
    "the machine identity needs both a client ID and a client secret";

/// What a check is refused with when the process has no secret source (D5; `--demo`, blueprint
/// A-5).
pub const NO_SOURCE_TO_CHECK: &str = "this session has no secret provider to check";

/// What a scope check of a project with no provider answers (D8).
pub const NO_PROVIDER_TO_CHECK: &str = "the project has no secret provider";

/// A secret on its way through [`StoreRequest`], which derives `Debug` and `Clone` (MOD-10 M4
/// D9): cloned zeroizing, wiped on drop, printed as `Redacted(<redacted>)`. No `Display`,
/// `Serialize` or `PartialEq`.
#[derive(Clone)]
pub struct Redacted(Zeroizing<String>);

impl Redacted {
    /// Wraps `text`, taking its allocation (no copy).
    #[must_use]
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// The secret. For the keyring write only; never format it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether it is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl core::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Redacted(<redacted>)")
    }
}

/// A machine identity as typed in `Settings > Secrets` (D4). `Clone`, unlike
/// [`MachineIdentity`], so it can ride a `StoreRequest`. `Debug` is `IdentityEntry(<redacted>)`:
/// the client ID is not shown either (OQ-4).
#[derive(Clone)]
pub struct IdentityEntry {
    client_id: Zeroizing<String>,
    client_secret: Redacted,
}

impl IdentityEntry {
    /// Both halves; the section trims them and refuses a blank one before building this.
    #[must_use]
    pub fn new(client_id: String, client_secret: Redacted) -> Self {
        Self {
            client_id: Zeroizing::new(client_id),
            client_secret,
        }
    }

    /// The client ID.
    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    /// The client secret. For the keyring write only.
    #[must_use]
    pub fn expose_client_secret(&self) -> &str {
        self.client_secret.expose()
    }

    /// The worker's copy for `htui_store::secret::set_machine_identity`: both halves wiped on
    /// drop.
    pub(crate) fn to_identity(&self) -> MachineIdentity {
        MachineIdentity::new(self.client_id(), self.expose_client_secret())
    }
}

impl core::fmt::Debug for IdentityEntry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("IdentityEntry(<redacted>)")
    }
}

/// What `Settings > Secrets` shows of the keyring (D2). Never a value: the URL is its normalised
/// form (no user info, query or fragment), the identity a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretsSnapshot {
    /// The Infisical base URL row.
    pub url: UrlState,
    /// The machine identity row.
    pub identity: IdentityState,
}

/// The URL row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlState {
    /// `Backend::Memory`: no keyring consulted (D10).
    NotApplicable,
    /// The keyring answered and holds no URL.
    NotStored,
    /// The stored URL through `htui_secrets::normalise_base_url`.
    Stored(String),
    /// Stored, but normalisation refuses it: its `SecretError` sentence, never the stored text.
    Unusable(String),
    /// The seam's sentence, without `StoreError`'s prefix (`connection::seam_sentence`'s rule).
    Unreadable(String),
}

/// The identity row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityState {
    /// `Backend::Memory`: no keyring consulted (D10).
    NotApplicable,
    /// The keyring answered and holds neither half.
    NotStored,
    /// Both halves are stored.
    Stored,
    /// One half present: the seam's sentence, which names the missing slot (A-7).
    HalfStored(String),
    /// The keyring could not be read: the seam's sentence. Never shown as `NotStored`.
    Unreadable(String),
}

/// One check's answer (D5, D8; A-10). Counts and sentences only: never a key name or value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretCheck {
    /// [`StoreRequest::CheckSecretProvider`]'s: reachability plus a fresh login.
    Provider {
        /// When the check finished.
        at: DateTime<Utc>,
        /// The provider's health, or why there was none.
        outcome: std::result::Result<ProviderHealth, SecretError>,
    },
    /// [`StoreRequest::CheckSecretScope`]'s: how many keys the project's scope shows.
    Scope {
        /// The project checked.
        project: ProjectId,
        /// When the check finished.
        at: DateTime<Utc>,
        /// The key count, or why there was none.
        outcome: std::result::Result<usize, SecretError>,
    },
}

/// The read (D2).
///
/// On [`Backend::Memory`] nothing is read and both rows are [`UrlState::NotApplicable`] /
/// [`IdentityState::NotApplicable`] (D10). Otherwise **one** `spawn_blocking` reads the URL and
/// the identity and classifies both inside the closure, so the identity is dropped, wiped, there.
///
/// # Errors
///
/// [`StoreError::Backend`] when the blocking task fails to join. An unreadable keyring is a row,
/// not an error.
pub async fn snapshot(backend: &Backend) -> Result<SecretsSnapshot> {
    if matches!(backend, Backend::Memory(_)) {
        return Ok(SecretsSnapshot {
            url: UrlState::NotApplicable,
            identity: IdentityState::NotApplicable,
        });
    }
    tokio::task::spawn_blocking(|| SecretsSnapshot {
        url: url_state(secret::get_infisical_url()),
        identity: identity_state(secret::get_machine_identity()),
    })
    .await
    .map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))
}

/// The URL row from what the keyring answered.
fn url_state(read: Result<Option<String>>) -> UrlState {
    match read {
        Ok(None) => UrlState::NotStored,
        Ok(Some(raw)) => {
            // Wiped on drop; never cloned out. Normalisation never echoes it.
            let raw = Zeroizing::new(raw);
            match htui_secrets::normalise_base_url(&raw) {
                Ok(normalised) => UrlState::Stored(normalised),
                Err(err) => UrlState::Unusable(err.to_string()),
            }
        }
        Err(err) => UrlState::Unreadable(seam_sentence(&err)),
    }
}

/// The identity row from what the keyring answered. The identity itself is dropped here.
fn identity_state(read: Result<Option<MachineIdentity>>) -> IdentityState {
    match read {
        Ok(Some(_)) => IdentityState::Stored,
        Ok(None) => IdentityState::NotStored,
        Err(StoreError::Backend(message)) if message.starts_with(secret::HALF_STORED_IDENTITY) => {
            IdentityState::HalfStored(message)
        }
        Err(err) => IdentityState::Unreadable(seam_sentence(&err)),
    }
}

/// The seam's own sentence, without the `StoreError` wrapper the shell puts on a store failure: a
/// private copy of `connection.rs`'s rule (blueprint B.4).
fn seam_sentence(err: &StoreError) -> String {
    match err {
        StoreError::Backend(message) => message.clone(),
        other => other.to_string(),
    }
}

/// `try_serve`'s arm for the read and the four keyring writes (D2).
///
/// A write on [`Backend::Memory`] is refused with [`DEMO_SESSION`] before anything else (D10). A
/// URL is normalised again (defensive: the section already did) and a refusal is `Failed` with
/// normalisation's sentence, which never echoes the URL. An identity with a blank half is
/// refused with [`IDENTITY_INCOMPLETE`]. Every write that lands bumps the keyring-write
/// generation (blueprint A-4) and answers a fresh snapshot under its own name
/// ([`StoreReply::SecretsWritten`], R1 M-1); the read answers [`StoreReply::Secrets`].
///
/// # Errors
///
/// A keyring write's [`StoreError::Backend`], which the caller renders as it renders
/// Connection's `SetDsn`; a failed join; and [`StoreError::Backend`] for a request that is not
/// one of this module's, which `try_serve` never sends here.
pub async fn serve(backend: &Backend, request: &StoreRequest) -> Result<StoreReply> {
    let demo = matches!(backend, Backend::Memory(_));
    match request {
        StoreRequest::SecretsInfo => return Ok(StoreReply::Secrets(snapshot(backend).await?)),
        StoreRequest::SetInfisicalUrl(_)
        | StoreRequest::ClearInfisicalUrl
        | StoreRequest::SetMachineIdentity(_)
        | StoreRequest::ClearMachineIdentity
            if demo =>
        {
            return Ok(refused(request, DEMO_SESSION.to_owned()));
        }
        StoreRequest::SetInfisicalUrl(url) => {
            let normalised = match htui_secrets::normalise_base_url(url) {
                Ok(normalised) => normalised,
                Err(err) => return Ok(refused(request, err.to_string())),
            };
            keyring_write(move || secret::set_infisical_url(&normalised)).await?;
        }
        StoreRequest::ClearInfisicalUrl => keyring_write(secret::clear_infisical_url).await?,
        StoreRequest::SetMachineIdentity(entry) => {
            if entry.client_id().trim().is_empty() || entry.expose_client_secret().trim().is_empty()
            {
                return Ok(refused(request, IDENTITY_INCOMPLETE.to_owned()));
            }
            let entry = entry.clone();
            keyring_write(move || secret::set_machine_identity(&entry.to_identity())).await?;
        }
        StoreRequest::ClearMachineIdentity => {
            keyring_write(secret::clear_machine_identity).await?;
        }
        other => {
            return Err(StoreError::Backend(format!(
                "not a secrets request: {}",
                other.name()
            )));
        }
    }
    Ok(StoreReply::SecretsWritten {
        request: request.name(),
        snapshot: snapshot(backend).await?,
    })
}

/// `request`'s `Failed` with `message`.
fn refused(request: &StoreRequest, message: String) -> StoreReply {
    StoreReply::Failed {
        request: request.name(),
        message,
    }
}

/// Runs one keyring write on a blocking thread and, when it lands, notes it so the next
/// `provider()` builds afresh (blueprint A-4).
async fn keyring_write<F>(write: F) -> Result<()>
where
    F: FnOnce() -> Result<()> + Send + 'static,
{
    tokio::task::spawn_blocking(write)
        .await
        .map_err(|err| StoreError::Backend(format!("keyring task failed: {err}")))??;
    crate::secrets::note_keyring_write();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "zq7-client-secret-0123456789";
    const CLIENT_ID: &str = "cid-typed-1";

    #[test]
    fn redacted_debug_is_fixed_and_clone_keeps_the_text() {
        let redacted = Redacted::new(SECRET.to_owned());
        assert_eq!(format!("{redacted:?}"), "Redacted(<redacted>)");
        let copy = redacted.clone();
        assert_eq!(copy.expose(), SECRET);
        assert!(!copy.is_empty());
        assert!(Redacted::new(String::new()).is_empty());
        assert_eq!(format!("{copy:?}"), "Redacted(<redacted>)");
    }

    #[test]
    fn identity_entry_debug_hides_both_halves() {
        let entry = IdentityEntry::new(CLIENT_ID.to_owned(), Redacted::new(SECRET.to_owned()));
        let shown = format!("{entry:?} {:?}", entry.clone());
        assert_eq!(
            format!("{entry:?}"),
            "IdentityEntry(<redacted>)",
            "neither half is printed"
        );
        assert!(
            !shown.contains(SECRET) && !shown.contains(CLIENT_ID),
            "{shown}"
        );
        assert_eq!(entry.client_id(), CLIENT_ID);
        assert_eq!(entry.expose_client_secret(), SECRET);
        let identity = entry.to_identity();
        assert_eq!(identity.client_id(), CLIENT_ID);
        assert_eq!(identity.client_secret(), SECRET);
    }

    #[test]
    fn the_request_names_are_the_module_constants() {
        let requests = [
            StoreRequest::SecretsInfo,
            StoreRequest::SetInfisicalUrl("https://x.example".to_owned()),
            StoreRequest::ClearInfisicalUrl,
            StoreRequest::SetMachineIdentity(IdentityEntry::new(
                CLIENT_ID.to_owned(),
                Redacted::new(SECRET.to_owned()),
            )),
            StoreRequest::ClearMachineIdentity,
        ];
        let names: Vec<&str> = requests.iter().map(StoreRequest::name).collect();
        assert_eq!(names, REQUEST_NAMES);
        assert_eq!(READ_NAME, "secrets_info");
        assert_eq!(
            StoreRequest::CheckSecretProvider.name(),
            CHECK_SECRET_PROVIDER
        );
        assert_eq!(
            StoreRequest::CheckSecretScope {
                project: ProjectId::new(),
            }
            .name(),
            CHECK_SECRET_SCOPE
        );
        assert_eq!(
            StoreRequest::SetProjectSecretScope {
                id: ProjectId::new(),
                expected: Utc::now(),
                scope: None,
            }
            .name(),
            SET_PROJECT_SECRET_SCOPE
        );
    }
}

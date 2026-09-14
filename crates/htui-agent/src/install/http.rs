//! The one file in the workspace that names `reqwest` (plan MOD-20 D9).
//!
//! Two clients are built from one provider install, because `reqwest::ClientBuilder::timeout` is
//! a **per-client** setting and this item needs two different shapes (blueprint P-7): the
//! `short` client bounds a whole request — the registry `GET` and the archive `HEAD`, both of
//! which are small and must not hold the pre-flight open — while the `long` client bounds only
//! the connect and the gap *between* body chunks, so a 682 MB archive is not cut off at fifteen
//! seconds and a captive portal that accepts the socket and then says nothing still fails in a
//! minute rather than never (hazard H-13).
//!
//! Everything above this module speaks in `HeadInfo`, `RegistryFetch` and `HttpError`, so
//! the day the client changes, the pre-flight and the fetch do not.

use std::sync::Once;
use std::time::Duration;

use reqwest::header::{CACHE_CONTROL, CONTENT_LENGTH, ETAG, HeaderMap, IF_NONE_MATCH};

use super::{InstallConfig, InstallError};

/// `htui/<version>`, so a CDN log line names the client and not "unknown".
const USER_AGENT: &str = concat!("htui/", env!("CARGO_PKG_VERSION"));

/// The most of a registry document this client will hold in memory.
///
/// The published document is about 40 KB and the whole of it is read into a `Vec` before anything
/// parses it, so the size is an attacker's — or a misrouted CDN's — to choose unless a number here
/// says otherwise. Four megabytes is a hundred times the real document and still nothing beside
/// the archives this installer downloads on purpose.
const REGISTRY_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Guards [`install_crypto_provider`]: the process installs one default provider, once.
static PROVIDER: Once = Once::new();

/// Installs `ring` as this process's rustls provider, ignoring the failure that means it is
/// already installed.
///
/// Plan D9 as amended by the fact-check: `reqwest`'s `rustls-no-provider` feature does **not**
/// infer the single provider the build enabled. With `rustls/ring` on and nothing else,
/// `Client::builder().build()` still panics with *"No rustls crypto provider is configured. When
/// using the `rustls-no-provider` feature you must install a crypto provider before building a
/// Client"*. So the call is explicit and it happens before either client is built.
///
/// `install_default`'s `Err` is dropped on purpose: it means some other crate in this process —
/// `sqlx`, a test binary, a second `Installer` — already installed one, and that is the outcome
/// this function wants. The `Once` is belt and braces around the same idea.
///
/// Hazard H-1: `a_client_builds_after_the_provider_is_installed` in `tests/install.rs` is what
/// turns the day someone deletes this call into a red test rather than a panic under the user's
/// first `i`.
fn install_crypto_provider() {
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// The installer's two HTTP clients (plan D9, blueprint P-7).
#[derive(Debug)]
pub(crate) struct HttpClient {
    /// Total-timeout client: the registry `GET` and the archive `HEAD`.
    short: reqwest::Client,
    /// Idle-timeout client: the archive body, whose total time is the user's to spend.
    long: reqwest::Client,
}

impl HttpClient {
    /// Both clients, after the provider install.
    ///
    /// # Errors
    ///
    /// [`InstallError::Http`] when either builder refuses — a TLS backend that will not
    /// initialise, or a system proxy configuration that does not parse. It is an error and not a
    /// panic because `AgentRuntime::production()` must not open a socket or die before the user
    /// has asked for anything (blueprint P-13).
    pub(crate) fn new(config: &InstallConfig) -> Result<Self, InstallError> {
        install_crypto_provider();
        let short = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(config.registry_timeout)
            .connect_timeout(config.connect_timeout)
            .build()
            .map_err(|error| InstallError::Http(error.to_string()))?;
        let long = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .build()
            .map_err(|error| InstallError::Http(error.to_string()))?;
        Ok(Self { short, long })
    }

    /// `GET url`, conditional when the cache has an `ETag` to offer (plan D12).
    ///
    /// A `304` is [`RegistryFetch::NotModified`] and carries no body: the caller keeps what it
    /// cached and only refreshes when it last asked. Any other non-2xx is an error, because a
    /// `404` document is not a registry.
    ///
    /// The body is bounded by [`REGISTRY_MAX_BYTES`] — twice: the declared `content-length` is
    /// refused before a byte is read, and the count is checked again as the chunks arrive, because
    /// a response that declares nothing can still send everything.
    ///
    /// # Errors
    ///
    /// [`HttpError`] for a refused connection, a timeout, a status that is neither 2xx nor 304, or
    /// a document larger than this client will hold.
    pub(crate) async fn get_registry(
        &self,
        url: &str,
        if_none_match: Option<&str>,
    ) -> Result<RegistryFetch, HttpError> {
        let mut request = self.short.get(url);
        if let Some(tag) = if_none_match {
            request = request.header(IF_NONE_MATCH, tag);
        }
        let mut response = request.send().await.map_err(HttpError::from_reqwest)?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_MODIFIED {
            return Ok(RegistryFetch::NotModified);
        }
        if !status.is_success() {
            return Err(HttpError::status(url, status.as_u16()));
        }
        let etag = header_text(response.headers(), &ETAG);
        let max_age = max_age_header(response.headers());
        if content_length_header(response.headers()).is_some_and(|size| size > REGISTRY_MAX_BYTES) {
            return Err(HttpError::too_big(url));
        }
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(HttpError::from_reqwest)? {
            if bytes.len() as u64 + chunk.len() as u64 > REGISTRY_MAX_BYTES {
                return Err(HttpError::too_big(url));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(RegistryFetch::Body {
            bytes,
            etag,
            max_age,
        })
    }

    /// `HEAD url` on the short client, redirects followed by the default policy.
    ///
    /// Answers even for a non-2xx status: the pre-flight treats "the archive URL does not answer"
    /// as *size unknown* and still produces a plan, because a CDN that refuses a `HEAD` is not a
    /// reason to refuse the install the user asked for (plan D11).
    ///
    /// # Errors
    ///
    /// [`HttpError`] when the request never completed at all.
    pub(crate) async fn head(&self, url: &str) -> Result<HeadInfo, HttpError> {
        let response = self
            .short
            .head(url)
            .send()
            .await
            .map_err(HttpError::from_reqwest)?;
        Ok(HeadInfo {
            status: response.status().as_u16(),
            content_length: content_length_header(response.headers()),
        })
    }

    /// `GET url` on the **long** client, with the body left unread (plan D9).
    ///
    /// The headers come back at once and the body arrives chunk by chunk through
    /// [`ByteStream::chunk`], which is what the progress cell counts and what a cancellation
    /// interrupts: dropping the future between two chunks closes the connection now rather than
    /// at the end of a 682 MB transfer. Each chunk is bounded by the long client's `read_timeout`,
    /// so a captive portal that accepts the socket and then says nothing fails in a minute rather
    /// than never (hazard H-13).
    ///
    /// # Errors
    ///
    /// [`HttpError`] for a refused connection, a timeout, or any non-2xx status — a `404` body is
    /// not an archive, and unpacking one would be worse than failing.
    pub(crate) async fn stream(&self, url: &str) -> Result<(HeadInfo, ByteStream), HttpError> {
        let response = self
            .long
            .get(url)
            .send()
            .await
            .map_err(HttpError::from_reqwest)?;
        let status = response.status();
        if !status.is_success() {
            return Err(HttpError::status(url, status.as_u16()));
        }
        let info = HeadInfo {
            status: status.as_u16(),
            content_length: content_length_header(response.headers()),
        };
        Ok((info, ByteStream { response }))
    }
}

/// A response body, one chunk at a time.
///
/// A struct rather than a `Stream`, so this file keeps its promise to be the only one that names
/// `reqwest`: an `impl Stream` return would either leak `reqwest::Error` through the item type or
/// need a `futures` dependency to box, and neither buys anything a `while let Some(chunk)` loop
/// does not already have.
#[derive(Debug)]
pub(crate) struct ByteStream {
    response: reqwest::Response,
}

impl ByteStream {
    /// The next chunk, or `None` at the end of the body.
    ///
    /// The chunk is handed over without a copy, which is the whole reason `bytes` is a direct
    /// dependency: a `Vec<u8>` here would copy every chunk of every archive.
    ///
    /// # Errors
    ///
    /// [`HttpError`] when the connection drops or a chunk does not arrive inside the long client's
    /// `read_timeout`.
    pub(crate) async fn chunk(&mut self) -> Result<Option<bytes::Bytes>, HttpError> {
        self.response.chunk().await.map_err(HttpError::from_reqwest)
    }
}

/// What a conditional registry `GET` answered.
#[derive(Debug)]
pub(crate) enum RegistryFetch {
    /// `304`: the cached document is current and its `fetched_at` is all that moves.
    NotModified,
    /// A document, with whatever caching the CDN attached to it.
    Body {
        /// The raw bytes, so the cache stores exactly what was parsed.
        bytes: Vec<u8>,
        /// `ETag`, for the next conditional request.
        etag: Option<String>,
        /// `cache-control: max-age`, when the CDN sent one.
        max_age: Option<Duration>,
    },
}

/// What a `HEAD` of the archive URL answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HeadInfo {
    /// The final status after redirects.
    pub(crate) status: u16,
    /// The `content-length` **header** — see [`content_length_header`].
    pub(crate) content_length: Option<u64>,
}

/// The archive's size, read from the `content-length` header and nowhere else.
///
/// This function exists because of the trap it closes (hazard H-2). `reqwest::Response::
/// content_length()` reports the length of the *body*, and a `HEAD` response has no body: against
/// the real 682 MB archive it answers `Some(0)` while the header says `681969407`. Reading it
/// would make every consent pane say "0 bytes", the D11 disk check never refuse, and the number
/// the user is shown a lie. So the header is the only source, and this is the only reader.
///
/// A header that is absent, not ASCII, or not a `u64` is `None` — *size unknown*, which the
/// pre-flight states rather than refuses.
#[must_use]
pub fn content_length_header(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// `cache-control: …max-age=N…` as a [`Duration`], or `None` when the header says nothing useful.
///
/// Directives are comma-separated and case-insensitive; anything but `max-age` is ignored, which
/// includes `s-maxage` — an intermediary's number, not this client's.
pub(crate) fn max_age_header(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(CACHE_CONTROL)?.to_str().ok()?;
    for directive in value.split(',') {
        let directive = directive.trim();
        let Some((name, seconds)) = directive.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("max-age") {
            return seconds.trim().parse().ok().map(Duration::from_secs);
        }
    }
    None
}

/// One header as an owned `String`, when it is present and ASCII.
fn header_text(headers: &HeaderMap, name: &reqwest::header::HeaderName) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_owned)
}

/// A transport-level failure, reduced to the one thing the caller renders.
///
/// The status, when there was one, is folded into the message rather than carried beside it:
/// every caller either shows the text or falls back to the cache, and none of them branches on
/// the number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HttpError {
    /// What to show the user, already a sentence.
    pub(crate) message: String,
}

impl HttpError {
    /// A request that never completed: refused, timed out, or a TLS handshake that failed.
    fn from_reqwest(error: reqwest::Error) -> Self {
        Self {
            message: error.to_string(),
        }
    }

    /// A request that completed with a status the caller cannot use.
    fn status(url: &str, status: u16) -> Self {
        Self {
            message: format!("{url} answered HTTP {status}"),
        }
    }

    /// A document larger than [`REGISTRY_MAX_BYTES`], refused by the header or by the count.
    fn too_big(url: &str) -> Self {
        Self {
            message: format!(
                "{url} answered more than {REGISTRY_MAX_BYTES} bytes; that is not the registry \
                 document"
            ),
        }
    }
}

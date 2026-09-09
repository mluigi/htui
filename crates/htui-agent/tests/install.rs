//! The registry reader and the install pre-flight (plan MOD-20 T4, D9, D11–D13, D20).
//!
//! Every case here runs **offline**, against a hand-rolled HTTP/1.1 responder on a loopback
//! [`TcpListener`], and every case writes under a `tempdir`. Both are deliberate. The responder is
//! what makes plan D13 — *no archive body byte before consent* — an assertion rather than a hope:
//! it records every request line it is sent, so a speculative range request added by a future
//! change fails `the_pre_flight_issues_one_get_and_one_head_and_nothing_else` immediately. The
//! temporary root is what keeps a `cargo test` on the maintainer's own box from writing into
//! `~/.local/share/htui/agents`, which [`InstallConfig::root_override`] exists for: `set_var` is
//! `unsafe` and forbidden here, so the install root travels as data.
//!
//! There is deliberately **no** `#[tokio::test(start_paused = true)]` in this file (blueprint
//! H-18): paused time auto-advances past a fifteen-second client timeout while the socket is not
//! yet readable, and every fetch case would fail for a reason that has nothing to do with the
//! code. Time that has to move in a test moves through `plan()`'s `now` argument instead.
//!
//! This file may name a vendor; the `install/` sources may not (`R-AGT-5`, enforced by
//! `the_installer_names_no_vendor` in `tests/extensibility.rs`).

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use chrono::{TimeDelta, Utc};
use htui_agent::acp::{AcpIo, Handshake, handshake};
use htui_agent::driver::DriverFuture;
use htui_agent::error::DriverError;
use htui_agent::install::http::content_length_header;
use htui_agent::install::layout::{Layout, nonce};
use htui_agent::install::{
    ArchiveFormat, Consent, InstallConfig, InstallError, InstallJob, InstallOutcome, InstallPhase,
    InstallProgress, InstallRecord, Installer, Manifest, PlanError, STAGING_MAX_AGE, Throttle,
    install, plan,
};
use htui_agent::launch::{AcpSettings, ResolvedLaunch};
use htui_agent::probe::{
    INSTALL_ROOT_VAR, ProbeContext, ProbeEnv, ProbeOutcome, ProbeSnapshot, ProbeStatus, Tier2,
    install_root, resolve_tool,
};
use htui_core::model::{Agent, AgentId, Billing, BoxId, Transport};
use reqwest::header::{CONTENT_LENGTH, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------------------------
// The fixture server
// ---------------------------------------------------------------------------------------------

/// One scripted answer: what the responder sends for one path.
#[derive(Debug, Clone, Default)]
struct Route {
    status: u16,
    /// Extra response headers. A `content-length` here **overrides** the body's real length,
    /// which is how the `HEAD` cases declare 682 MB while sending nothing.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// When set and the request's `If-None-Match` matches, the answer is `304` with no body.
    etag: Option<String>,
    /// How long to sit on the request before answering — the stall a client timeout has to cut.
    delay: Duration,
    /// When non-zero, the body is sent in two halves with this pause between them.
    ///
    /// It is what makes a cancellation land inside [`fetch::download`]'s `select!` rather than at
    /// the check that guards the top of its loop: without a gap between chunks, a body this small
    /// arrives whole before any token could be tripped, and the case would prove nothing about the
    /// arm hazard H-6 is written about.
    ///
    /// [`fetch::download`]: htui_agent::install::fetch::download
    chunk_delay: Duration,
}

/// One request the responder saw.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Recorded {
    /// `"<METHOD> <PATH>"`, which is what D13's assertion is written in terms of.
    line: String,
    if_none_match: Option<String>,
}

/// A loopback HTTP/1.1 responder with a scripted route table and a request recorder.
#[derive(Debug, Clone)]
struct Fixture {
    addr: SocketAddr,
    log: Arc<Mutex<Vec<Recorded>>>,
    routes: Arc<Mutex<HashMap<String, Route>>>,
}

impl Fixture {
    /// Binds an ephemeral port and starts accepting.
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral loopback port");
        let addr = listener.local_addr().expect("the bound address");
        let fixture = Self {
            addr,
            log: Arc::new(Mutex::new(Vec::new())),
            routes: Arc::new(Mutex::new(HashMap::new())),
        };
        let serving = fixture.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let serving = serving.clone();
                tokio::spawn(async move { serving.answer(stream).await });
            }
        });
        fixture
    }

    /// An address nothing listens on: bound, read, and immediately dropped.
    async fn refused() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("an ephemeral loopback port");
        listener.local_addr().expect("the bound address")
    }

    /// `http://127.0.0.1:<port>`, the value [`InstallConfig::registry_base`] takes.
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The absolute URL of one path on this server.
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// Scripts one path.
    fn route(&self, path: &str, route: Route) {
        self.routes
            .lock()
            .expect("the route table")
            .insert(path.to_owned(), route);
    }

    /// Every request line the responder has seen, in order.
    fn lines(&self) -> Vec<String> {
        self.log
            .lock()
            .expect("the recorder")
            .iter()
            .map(|seen| seen.line.clone())
            .collect()
    }

    /// The full recording, for the conditional-request case.
    fn recorded(&self) -> Vec<Recorded> {
        self.log.lock().expect("the recorder").clone()
    }

    /// Reads one request, records it, and writes the scripted answer.
    async fn answer(self, mut stream: TcpStream) {
        let Some(head) = read_head(&mut stream).await else {
            return;
        };
        let mut lines = head.lines();
        let Some(request_line) = lines.next() else {
            return;
        };
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let target = parts.next().unwrap_or_default().to_owned();
        let path = target
            .split(['?', '#'])
            .next()
            .unwrap_or(&target)
            .to_owned();
        let if_none_match = lines.find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("if-none-match")
                .then(|| value.trim().to_owned())
        });
        self.log.lock().expect("the recorder").push(Recorded {
            line: format!("{method} {path}"),
            if_none_match: if_none_match.clone(),
        });

        // The guard is dropped before the first `.await` below, on purpose: this file lives by the
        // same rule the worker does — no lock is ever held across a suspension point.
        let route = self
            .routes
            .lock()
            .expect("the route table")
            .get(&path)
            .cloned();
        let Some(route) = route else {
            let _ = stream.write_all(NOT_FOUND).await;
            return;
        };
        if !route.delay.is_zero() {
            tokio::time::sleep(route.delay).await;
        }

        let not_modified = route
            .etag
            .as_ref()
            .is_some_and(|tag| if_none_match.as_deref() == Some(tag.as_str()));
        let status = if not_modified { 304 } else { route.status };
        let declared = route
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.clone());

        let mut response = format!("HTTP/1.1 {status} {}\r\n", reason(status));
        response.push_str("connection: close\r\n");
        for (name, value) in &route.headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        if let Some(tag) = &route.etag {
            response.push_str(&format!("etag: {tag}\r\n"));
        }
        if !not_modified && declared.is_none() {
            response.push_str(&format!("content-length: {}\r\n", route.body.len()));
        }
        response.push_str("\r\n");

        let send_body = !not_modified && method != "HEAD";
        let _ = stream.write_all(response.as_bytes()).await;
        if send_body {
            if route.chunk_delay.is_zero() {
                let _ = stream.write_all(&route.body).await;
            } else {
                let (first, second) = route.body.split_at(route.body.len() / 2);
                let _ = stream.write_all(first).await;
                let _ = stream.flush().await;
                tokio::time::sleep(route.chunk_delay).await;
                let _ = stream.write_all(second).await;
            }
        }
        let _ = stream.flush().await;
    }
}

/// What an unscripted path answers.
const NOT_FOUND: &[u8] =
    b"HTTP/1.1 404 Not Found\r\nconnection: close\r\ncontent-length: 0\r\n\r\n";

/// The reason phrase for the four statuses this responder sends.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        304 => "Not Modified",
        404 => "Not Found",
        _ => "Internal Server Error",
    }
}

/// Reads bytes until the blank line that ends the request head.
async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if stream.read(&mut byte).await.ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
        if head.ends_with(b"\r\n\r\n") {
            return String::from_utf8(head).ok();
        }
        if head.len() > 16 * 1024 {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Fixtures: the pinned registry, the rows, the box
// ---------------------------------------------------------------------------------------------

/// The archive size the real `antigravity-acp` `linux-x86_64` zip reports, to the byte.
///
/// The number is the point: `reqwest::Response::content_length()` answers `Some(0)` for the same
/// `HEAD`, because a `HEAD` has no body (hazard H-2).
const REAL_ARCHIVE_BYTES: u64 = 681_969_407;

/// `tests/fixtures/registry.json`: the two live entries, verbatim, as fetched on 2026-09-09.
fn pinned_registry() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/registry.json");
    let text = std::fs::read_to_string(path).expect("the pinned registry is readable");
    serde_json::from_str(&text).expect("the pinned registry is JSON")
}

/// The pinned document with one entry's archive URL pointed at the fixture server.
///
/// The document is otherwise untouched: the `cmd`, `args`, `sha256`, `license` and `license_url`
/// a plan is asserted against are the registry's own words, not the test's.
fn with_archive(mut document: Value, id: &str, platform: &str, url: &str) -> Value {
    let agents = document["agents"]
        .as_array_mut()
        .expect("the document lists agents");
    for agent in agents {
        if agent["id"] == json!(id) {
            agent["distribution"]["binary"][platform]["archive"] = json!(url);
        }
    }
    document
}

/// A row that declares the first entry as its source, with the seed's own glob shape.
fn agy_row() -> Agent {
    row(
        "agy",
        json!({
            "agy_acp_server": {
                "kind": "glob",
                "patterns": [],
                "platform": {
                    "linux-x86_64": {
                        "patterns": ["%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.par"],
                        "args": ["--uid="],
                    },
                    "darwin-aarch64": {
                        "patterns": ["%HTUI_AGENTS_ROOT%/antigravity-acp/*/agy_acp_server.par"],
                    },
                },
            },
        }),
        Some(json!({
            "source": "acp_registry",
            "id": "antigravity-acp",
            "tool": "agy_acp_server",
        })),
    )
}

/// A row that declares the second entry as its source — a different vendor, a different archive
/// shape, a published digest, and not one line of code that knows either fact.
fn amp_row() -> Agent {
    row(
        "amp",
        json!({
            "amp_acp": {
                "kind": "glob",
                "patterns": [],
                "platform": {
                    "linux-x86_64": {
                        "patterns": ["%HTUI_AGENTS_ROOT%/amp-acp/*/amp-acp"],
                    },
                },
            },
        }),
        Some(json!({ "source": "acp_registry", "id": "amp-acp", "tool": "amp_acp" })),
    )
}

/// A row whose `discovery` declares tools and no source at all.
fn row(name: &str, tools: Value, install: Option<Value>) -> Agent {
    let mut discovery = json!({ "tools": tools, "handshake": true });
    if let Some(install) = install {
        discovery["install"] = install;
    }
    let now = Utc::now();
    Agent {
        id: AgentId::new(),
        name: name.to_owned(),
        transport: Transport::Acp,
        launch: json!({
            "command": "${tool}",
            "args": [],
            "env": {},
            "discovery": discovery,
        }),
        models: Vec::new(),
        default_model: None,
        billing: Billing::Subscription,
        enabled: true,
        settings: json!({}),
        created_at: now,
        updated_at: now,
    }
}

/// The box as a value, with the install root injected and **asserted** to be under `tmp`.
///
/// The assertion is not ceremony: without the override, `ProbeEnv::host` seeds the token from the
/// maintainer's own `dirs::data_local_dir()`, and a `plan()` that read it would list the real
/// installed versions and write a real registry cache. It runs before any case touches disk.
fn boxed(config: &InstallConfig, tmp: &Path, platform: &str) -> ProbeEnv {
    let env = config.apply_to(ProbeEnv {
        cwd: tmp.join("cwd"),
        platform: platform.to_owned(),
        home: Some(tmp.join("home")),
        vars: BTreeMap::new(),
        versions: false,
        version_timeout: Duration::from_secs(1),
    });
    let root = install_root(&env).expect("the override seeded the install root");
    assert!(
        root.starts_with(tmp),
        "{INSTALL_ROOT_VAR} must point inside the tempdir, not at {}",
        root.display()
    );
    env
}

/// A `GET`-able registry document with the CDN's own caching headers.
fn registry_route(document: &Value, max_age: u64, etag: &str) -> Route {
    Route {
        status: 200,
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("cache-control".to_owned(), format!("max-age={max_age}")),
        ],
        body: serde_json::to_vec(document).expect("the document serialises"),
        etag: Some(etag.to_owned()),
        ..Route::default()
    }
}

/// A `HEAD`-able archive that declares `size` and sends nothing.
fn archive_route(size: u64) -> Route {
    Route {
        status: 200,
        headers: vec![("content-length".to_owned(), size.to_string())],
        ..Route::default()
    }
}

/// The fixture server, an installer pointed at it, and a temporary install root.
struct Rig {
    _tmp: tempfile::TempDir,
    fixture: Fixture,
    installer: Installer,
    config: InstallConfig,
}

impl Rig {
    /// A rig whose `/registry.json` serves `document` and whose `path` is a `HEAD`-able archive.
    async fn new(document: Value, archive_path: &str, size: u64) -> Self {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let fixture = Fixture::start().await;
        let document = with_archive(
            document,
            "antigravity-acp",
            "linux-x86_64",
            &fixture.url(archive_path),
        );
        let document = with_archive(
            document,
            "amp-acp",
            "linux-x86_64",
            &fixture.url(archive_path),
        );
        fixture.route("/registry.json", registry_route(&document, 300, "\"v1\""));
        fixture.route(archive_path, archive_route(size));
        let config = InstallConfig::new(fixture.base(), Some(tmp.path().join("agents")));
        let installer = Installer::new(config.clone()).expect("a client builds");
        Self {
            _tmp: tmp,
            fixture,
            installer,
            config,
        }
    }

    /// The default rig: the pinned document, a `.zip` archive declaring the real size.
    async fn pinned() -> Self {
        Self::new(pinned_registry(), "/archive.zip", REAL_ARCHIVE_BYTES).await
    }

    fn tmp(&self) -> &Path {
        self._tmp.path()
    }
}

// ---------------------------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------------------------

/// The pinned registry parses into the reader's own types, with the two shapes the pre-flight has
/// to tell apart: an entry that publishes no digest and one that publishes one per platform.
#[test]
fn the_pinned_registry_parses_into_both_published_shapes() {
    let document: htui_agent::RegistryDocument =
        serde_json::from_value(pinned_registry()).expect("the pinned registry parses");
    assert_eq!(document.version, "1.0.0");

    let first = document.agent("antigravity-acp").expect("the first entry");
    assert_eq!(first.version, "1.1.1");
    assert_eq!(first.license.as_deref(), Some("proprietary"));
    assert_eq!(first.distribution.binary.len(), 5);
    assert!(
        first
            .distribution
            .binary
            .values()
            .all(|entry| entry.sha256.is_none()),
        "this entry publishes no digest on any platform, which is the wording D13 has a sentence \
         for"
    );

    let second = document.agent("amp-acp").expect("the second entry");
    assert_eq!(second.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(second.distribution.binary.len(), 5);
    assert!(
        second
            .distribution
            .binary
            .values()
            .all(|entry| entry.sha256.is_some()),
        "this entry publishes a digest on every platform"
    );
}

// ---------------------------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------------------------

/// The pre-flight answers every coordinate the consent text needs, from the document alone.
#[tokio::test]
async fn a_plan_answers_every_coordinate_for_this_platform() {
    let rig = Rig::pinned().await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");

    let plan = plan(&rig.installer, &agy_row(), &env, Utc::now())
        .await
        .expect("the pre-flight succeeds");

    assert_eq!(plan.registry_id, "antigravity-acp");
    assert_eq!(plan.registry_name, "Google Antigravity");
    assert_eq!(plan.version, "1.1.1");
    assert_eq!(plan.platform, "linux-x86_64");
    assert_eq!(plan.archive_url, rig.fixture.url("/archive.zip"));
    assert_eq!(plan.format, ArchiveFormat::Zip);
    assert_eq!(plan.cmd, "./agy_acp_server.par");
    assert_eq!(plan.args, vec!["--uid=".to_owned()]);
    assert_eq!(plan.license.as_deref(), Some("proprietary"));
    assert_eq!(
        plan.license_url.as_deref(),
        Some("https://antigravity.google/terms")
    );
    assert_eq!(plan.sha256, None);
    assert_eq!(plan.content_length, Some(REAL_ARCHIVE_BYTES));
    assert_eq!(plan.need_bytes, Some(REAL_ARCHIVE_BYTES * 4));
    // D11's free space, from the filesystem the temporary root sits on (T5 wired `fs4` here). The
    // root itself does not exist yet — this is a first install — so the number comes from the
    // nearest ancestor that does, and the plan exists at all only because it is above the need.
    assert!(
        plan.available_bytes
            .is_some_and(|free| free >= plan.need_bytes.unwrap_or_default()),
        "the pre-flight asks the filesystem itself: {:?} free against {:?} needed",
        plan.available_bytes,
        plan.need_bytes
    );
    assert!(!plan.args_differ, "the seed row copies the registry's args");
    assert!(plan.existing_versions.is_empty());
    assert_eq!(
        plan.install_dir,
        rig.tmp().join("agents/antigravity-acp/1.1.1")
    );
    assert_eq!(plan.registry_cached_age_secs, None);
}

/// The same code, a different entry: a `.tar.gz`, a published digest, no per-platform arguments.
/// Nothing in `install/` was told which of the two it was reading.
#[tokio::test]
async fn a_second_entry_plans_with_a_published_digest_and_a_tarball() {
    let rig = Rig::new(pinned_registry(), "/archive.tar.gz", 36_091_565).await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");

    let plan = plan(&rig.installer, &amp_row(), &env, Utc::now())
        .await
        .expect("the pre-flight succeeds");

    assert_eq!(plan.registry_id, "amp-acp");
    assert_eq!(plan.format, ArchiveFormat::TarGz);
    assert_eq!(plan.cmd, "./amp-acp");
    assert!(plan.args.is_empty());
    assert_eq!(plan.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(
        plan.sha256.as_deref(),
        Some("afaa50a152eb86a8ff21e354ded63fe2d21b730859692e3a60b2c4c9ef23df31")
    );
    assert_eq!(plan.content_length, Some(36_091_565));
    assert_eq!(
        plan.digest_sentence(),
        "sha256 published: verified before unpacking"
    );
}

/// A platform the entry does not publish for is a first-class answer, not a parse failure.
#[tokio::test]
async fn a_platform_the_entry_lacks_is_not_available_here() {
    let rig = Rig::pinned().await;
    let env = boxed(&rig.config, rig.tmp(), "darwin-x86_64");

    let error = plan(&rig.installer, &agy_row(), &env, Utc::now())
        .await
        .expect_err("this entry publishes nothing for that platform");

    assert!(
        matches!(&error, PlanError::NotAvailable { id, version, platform }
            if id == "antigravity-acp" && version == "1.1.1" && platform == "darwin-x86_64"),
        "unexpected refusal: {error:?}"
    );
}

/// An archive shape the unpacker does not know is refused **before** the `HEAD` — the whole point
/// of ordering the checks by what they cost.
#[tokio::test]
async fn an_unsupported_archive_is_refused_before_any_head() {
    let rig = Rig::new(pinned_registry(), "/archive.7z", 10).await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");

    let error = plan(&rig.installer, &agy_row(), &env, Utc::now())
        .await
        .expect_err("a .7z is not installable from this entry");

    assert!(
        matches!(&error, PlanError::Unsupported { url } if url.ends_with(".7z")),
        "unexpected refusal: {error:?}"
    );
    assert_eq!(
        rig.fixture.lines(),
        vec!["GET /registry.json".to_owned()],
        "the refusal must come before the server is asked about the archive at all"
    );
}

/// Plan D13, pinned: one registry `GET`, one archive `HEAD`, and nothing else before consent.
#[tokio::test]
async fn the_pre_flight_issues_one_get_and_one_head_and_nothing_else() {
    let rig = Rig::pinned().await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");

    plan(&rig.installer, &agy_row(), &env, Utc::now())
        .await
        .expect("the pre-flight succeeds");

    assert_eq!(
        rig.fixture.lines(),
        vec![
            "GET /registry.json".to_owned(),
            "HEAD /archive.zip".to_owned(),
        ],
        "`R-AGT-10` says nothing is fetched before the user is told; a body request here — even a \
         speculative range — is exactly what this assertion exists to catch"
    );
}

// ---------------------------------------------------------------------------------------------
// The cache
// ---------------------------------------------------------------------------------------------

/// Inside the CDN's own `max-age`, a second pre-flight asks the registry nothing.
#[tokio::test]
async fn a_second_plan_within_max_age_issues_no_registry_request() {
    let rig = Rig::pinned().await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");
    let now = Utc::now();

    plan(&rig.installer, &agy_row(), &env, now)
        .await
        .expect("the first pre-flight succeeds");
    let plan = plan(
        &rig.installer,
        &agy_row(),
        &env,
        now + TimeDelta::seconds(10),
    )
    .await
    .expect("the second pre-flight succeeds");

    assert_eq!(
        rig.fixture.lines(),
        vec![
            "GET /registry.json".to_owned(),
            "HEAD /archive.zip".to_owned(),
            "HEAD /archive.zip".to_owned(),
        ],
        "56 KB re-fetched on every `i` is the waste the cache exists to avoid"
    );
    assert_eq!(
        plan.registry_cached_age_secs, None,
        "a document the CDN still calls current is not `cached` in the consent text"
    );
}

/// Past `max-age`, the request is conditional and a `304` keeps the cached body.
#[tokio::test]
async fn past_max_age_a_conditional_get_answered_304_keeps_the_cache() {
    let rig = Rig::pinned().await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");
    let now = Utc::now();

    plan(&rig.installer, &agy_row(), &env, now)
        .await
        .expect("the first pre-flight succeeds");
    let plan = plan(
        &rig.installer,
        &agy_row(),
        &env,
        now + TimeDelta::seconds(301),
    )
    .await
    .expect("the second pre-flight succeeds from the cache the 304 confirmed");

    let recorded = rig.fixture.recorded();
    let conditional = recorded
        .iter()
        .filter(|seen| seen.line == "GET /registry.json")
        .collect::<Vec<_>>();
    assert_eq!(
        conditional.len(),
        2,
        "the stale document is re-validated once"
    );
    assert_eq!(conditional[0].if_none_match, None);
    assert_eq!(
        conditional[1].if_none_match.as_deref(),
        Some("\"v1\""),
        "the second read offers the ETag the first one was given"
    );
    assert_eq!(plan.version, "1.1.1");
    assert_eq!(plan.registry_cached_age_secs, None);
}

/// A registry that cannot be reached, with a warm cache, plans anyway and says how old the
/// document is — plan D12's "tell them, do not stop them".
#[tokio::test]
async fn a_registry_failure_with_a_warm_cache_plans_and_states_the_age() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let fixture = Fixture::start().await;
    let document = with_archive(
        pinned_registry(),
        "antigravity-acp",
        "linux-x86_64",
        &fixture.url("/archive.zip"),
    );
    // `max-age=0`: the cached copy is warm but never current, so the second read always asks.
    fixture.route("/registry.json", registry_route(&document, 0, "\"v1\""));
    fixture.route("/archive.zip", archive_route(REAL_ARCHIVE_BYTES));

    let root = tmp.path().join("agents");
    let live = InstallConfig::new(fixture.base(), Some(root.clone()));
    let now = Utc::now();
    plan(
        &Installer::new(live.clone()).expect("a client builds"),
        &agy_row(),
        &boxed(&live, tmp.path(), "linux-x86_64"),
        now,
    )
    .await
    .expect("the first pre-flight warms the cache");

    // The same root, a registry base nothing answers on. The archive `HEAD` still works, so the
    // only thing that failed is the document read.
    let dead = InstallConfig::new(format!("http://{}", Fixture::refused().await), Some(root));
    let plan = plan(
        &Installer::new(dead.clone()).expect("a client builds"),
        &agy_row(),
        &boxed(&dead, tmp.path(), "linux-x86_64"),
        now + TimeDelta::seconds(10),
    )
    .await
    .expect("a warm cache is a plan");

    assert_eq!(plan.version, "1.1.1");
    assert_eq!(plan.registry_cached_age_secs, Some(10));
    assert!(
        plan.consent_lines()
            .iter()
            .any(|line| line.starts_with("registry cached ")),
        "the consent text has to say the document is not today's: {:?}",
        plan.consent_lines()
    );
}

// ---------------------------------------------------------------------------------------------
// Degrading
// ---------------------------------------------------------------------------------------------

/// A `HEAD` that never answers costs the size and nothing else: the plan still exists, and the
/// consent text says the size is unknown rather than claiming zero.
#[tokio::test]
async fn a_head_that_stalls_leaves_the_size_unknown_and_still_plans() {
    let rig = Rig::pinned().await;
    rig.fixture.route(
        "/archive.zip",
        Route {
            delay: Duration::from_secs(30),
            ..archive_route(REAL_ARCHIVE_BYTES)
        },
    );
    // The registry `GET` on loopback takes microseconds; the stall does not.
    let mut config = rig.config.clone();
    config.registry_timeout = Duration::from_millis(400);
    let installer = Installer::new(config.clone()).expect("a client builds");
    let env = boxed(&config, rig.tmp(), "linux-x86_64");

    let plan = plan(&installer, &agy_row(), &env, Utc::now())
        .await
        .expect("an unanswerable HEAD is not a reason to refuse the install");

    assert_eq!(plan.content_length, None);
    assert_eq!(plan.need_bytes, None);
    assert!(
        plan.consent_lines()
            .iter()
            .any(|line| line.contains("size unknown")),
        "{:?}",
        plan.consent_lines()
    );
}

/// No network and no cache is plan D20's path: a refusal that carries the steps to do it by hand,
/// every word of them derived from the row and the helpers.
#[tokio::test]
async fn no_network_and_no_cache_answers_with_derived_manual_steps() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let config = InstallConfig::new(
        format!("http://{}", Fixture::refused().await),
        Some(tmp.path().join("agents")),
    );
    let installer = Installer::new(config.clone()).expect("a client builds");
    let env = boxed(&config, tmp.path(), "linux-x86_64");

    let error = plan(&installer, &agy_row(), &env, Utc::now())
        .await
        .expect_err("nothing to read and nothing cached");

    let PlanError::Network { manual, .. } = error else {
        panic!("a refused connection with no cache is the no-network path, not {error:?}");
    };
    assert_eq!(
        manual.registry_url,
        format!("{}/registry.json", config.registry_base)
    );
    assert_eq!(manual.id, "antigravity-acp");
    assert_eq!(manual.platform, "linux-x86_64");
    assert_eq!(manual.version, None);
    assert_eq!(
        manual.unpack_into,
        tmp.path().join("agents/antigravity-acp/<version>")
    );
    assert_eq!(manual.override_key, "HTUI_TOOL_AGY_ACP_SERVER");
    let lines = manual.lines();
    assert!(
        lines.iter().any(|line| line.contains(&manual.registry_url))
            && lines.iter().any(|line| line.contains("antigravity-acp"))
            && lines.iter().any(|line| line.contains("linux-x86_64"))
            && lines
                .iter()
                .any(|line| line.contains("HTUI_TOOL_AGY_ACP_SERVER")),
        "the fallback has to be usable on its own: {lines:?}"
    );
}

/// D11's arithmetic, refused before consent, with both numbers in the message.
#[tokio::test]
async fn free_space_below_four_times_the_archive_is_refused() {
    let rig = Rig::new(pinned_registry(), "/archive.zip", 1000).await;
    let mut config = rig.config.clone();
    config.disk_override = Some(1);
    let installer = Installer::new(config.clone()).expect("a client builds");
    let env = boxed(&config, rig.tmp(), "linux-x86_64");

    let error = plan(&installer, &agy_row(), &env, Utc::now())
        .await
        .expect_err("one byte free is not four thousand");

    assert!(
        matches!(
            &error,
            PlanError::Disk {
                need: 4000,
                available: 1,
                factor: 4,
                ..
            }
        ),
        "unexpected refusal: {error:?}"
    );
    assert!(
        error.to_string().contains("4000") && error.to_string().contains("1 available"),
        "the user is owed both numbers: {error}"
    );
}

/// The consent pane's disk line states the factor the plan was actually built with.
///
/// `need_bytes` comes from `config.headroom_factor` and the line used to print the
/// `DISK_HEADROOM_FACTOR` constant beside it, so a box configured with anything else was shown two
/// numbers that do not multiply out — the pane quietly misdescribing the arithmetic an install was
/// just allowed or refused on.
#[tokio::test]
async fn the_disk_line_states_the_factor_the_plan_was_built_with() {
    let rig = Rig::new(pinned_registry(), "/archive.zip", 1000).await;
    let mut config = rig.config.clone();
    config.headroom_factor = 2;
    config.disk_override = Some(1_000_000);
    let installer = Installer::new(config.clone()).expect("a client builds");
    let env = boxed(&config, rig.tmp(), "linux-x86_64");

    let plan = plan(&installer, &agy_row(), &env, Utc::now())
        .await
        .expect("two thousand bytes fit in a million");

    assert_eq!(
        plan.need_bytes,
        Some(2000),
        "2 × the archive, as configured"
    );
    let line = plan
        .consent_lines()
        .into_iter()
        .find(|line| line.starts_with("disk    "))
        .expect("the pane states the disk");
    assert!(
        line.contains("2× the archive"),
        "the factor in the sentence is the factor in the sum: {line}"
    );
}

/// A registry document larger than this client will hold is refused rather than read.
///
/// The document is fetched from a public CDN into a `Vec` before anything parses it, so its size
/// is whatever answers the request unless a number says otherwise. What the user gets instead is
/// plan D20's derived manual steps, which is what every other way of not having a document gets.
#[tokio::test]
async fn a_registry_document_larger_than_the_client_will_hold_is_refused() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let fixture = Fixture::start().await;
    // Valid JSON and a valid document — the refusal has to be about the size and not about a body
    // that would have failed to parse anyway.
    let mut document = with_archive(
        pinned_registry(),
        "antigravity-acp",
        "linux-x86_64",
        &fixture.url("/archive.zip"),
    );
    document["agents"][0]["name"] = json!("x".repeat(5 * 1024 * 1024));
    fixture.route("/registry.json", registry_route(&document, 300, "\"v1\""));
    fixture.route("/archive.zip", archive_route(1000));
    let config = InstallConfig::new(fixture.base(), Some(tmp.path().join("agents")));
    let installer = Installer::new(config.clone()).expect("a client builds");
    let env = boxed(&config, tmp.path(), "linux-x86_64");

    let error = plan(&installer, &agy_row(), &env, Utc::now())
        .await
        .expect_err("five megabytes is not the registry document");

    let PlanError::Network { message, manual } = &error else {
        panic!("a document that will not be read is a fetch that did not happen: {error:?}");
    };
    assert!(
        message.contains("more than"),
        "the message says what was refused: {message}"
    );
    assert!(
        manual.lines().iter().any(|line| line.contains("registry")),
        "and the user is left with the steps for doing it by hand"
    );
    assert_eq!(
        fixture.lines(),
        vec!["GET /registry.json".to_owned()],
        "the archive is never asked about: there is no entry to ask for"
    );
}

/// A row that declares no source says so, without touching the network.
#[tokio::test]
async fn a_row_that_declares_no_source_is_refused_by_name() {
    let rig = Rig::pinned().await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");
    let row = row("node-served", json!({}), None);

    let error = plan(&rig.installer, &row, &env, Utc::now())
        .await
        .expect_err("a NodePackage-served adapter cannot be installed from the app");

    assert!(
        matches!(&error, PlanError::NoSource { agent } if agent == "node-served"),
        "unexpected refusal: {error:?}"
    );
    assert!(
        rig.fixture.lines().is_empty(),
        "nothing to ask the registry"
    );
}

// ---------------------------------------------------------------------------------------------
// The two fact-check amendments
// ---------------------------------------------------------------------------------------------

/// A client builds — which it only does because `install/http.rs` installs the `ring` provider
/// first (plan D9 as amended, hazard H-1).
///
/// `reqwest`'s `rustls-no-provider` feature does **not** infer the single provider the build
/// enabled: with the call removed, `Client::builder().build()` panics with *"No rustls crypto
/// provider is configured. When using the `rustls-no-provider` feature you must install a crypto
/// provider before building a Client"*. That panic is not asserted here — catching it across a
/// `Once` would need `catch_unwind` and would poison the guard for every other case in this
/// binary — but this test is what turns its return into a red suite rather than a panic under the
/// user's first `i`.
#[test]
fn a_client_builds_after_the_provider_is_installed() {
    Installer::new(InstallConfig::default()).expect("the ring provider is installed first");
    Installer::new(InstallConfig::default()).expect("and installing it twice is not an error");
}

/// The archive's size comes from the `content-length` **header**, never from the response body
/// (hazard H-2).
///
/// Both halves of the trap are here: the header parser on its own, and a real `HEAD` through the
/// fixture whose declared size is 682 MB and whose body is empty. `Response::content_length()`
/// would answer `Some(0)` for that exchange, every consent pane would read "0 bytes", and D11's
/// disk check would never refuse anything.
#[tokio::test]
async fn the_size_comes_from_the_header_and_not_from_the_body() {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_LENGTH, HeaderValue::from_static("681969407"));
    assert_eq!(content_length_header(&headers), Some(REAL_ARCHIVE_BYTES));
    assert_eq!(content_length_header(&HeaderMap::new()), None);

    let rig = Rig::pinned().await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");
    let plan = plan(&rig.installer, &agy_row(), &env, Utc::now())
        .await
        .expect("the pre-flight succeeds");

    assert_eq!(plan.content_length, Some(REAL_ARCHIVE_BYTES));
    assert_eq!(
        rig.fixture.lines().last().map(String::as_str),
        Some("HEAD /archive.zip"),
        "the size came from a HEAD, whose body is empty by definition"
    );
}

// ---------------------------------------------------------------------------------------------
// The consent text
// ---------------------------------------------------------------------------------------------

/// The consent pane's wording, composed here so the Settings section composes nothing (plan D13).
#[tokio::test]
async fn the_consent_lines_state_the_terms_the_digest_and_the_disk() {
    let rig = Rig::pinned().await;
    let env = boxed(&rig.config, rig.tmp(), "linux-x86_64");
    let plan = plan(&rig.installer, &agy_row(), &env, Utc::now())
        .await
        .expect("the pre-flight succeeds");

    let lines = plan.consent_lines();
    assert_eq!(
        lines[0],
        "install Google Antigravity 1.1.1 for agy (linux-x86_64)"
    );
    assert!(lines[1].starts_with("from    http://"), "{:?}", lines[1]);
    assert!(lines[1].ends_with(" — 650.4 MB"), "{:?}", lines[1]);
    assert_eq!(lines[2], format!("into    {}", plan.install_dir.display()));
    assert_eq!(
        lines[3],
        "licence proprietary — https://antigravity.google/terms — y accepts these terms"
    );
    assert_eq!(
        lines[4],
        "digest  none published: htui cannot verify this download and will record what it receives"
    );
    assert!(lines[5].starts_with("disk    "), "{:?}", lines[5]);
    assert_eq!(lines[6], "existing none");
}

// ---------------------------------------------------------------------------------------------
// T5 fixtures: an install root, a tree, and the two archive shapes built from it
// ---------------------------------------------------------------------------------------------

/// The registry id the T5 cases install under.
///
/// It carries a dash on purpose: the sweep has to split `<id>-<version>-<nonce>` back into an id
/// and a version to know which directory a `.previous` belongs to, and an id without one would
/// prove nothing about that split (hazard H-7).
const ID: &str = "demo-acp";

/// The version those cases install.
const VERSION: &str = "1.1.1";

/// The command inside the archive, in a subdirectory and spelled the way the registry spells one.
///
/// The subdirectory is what makes "the whole tree, not the `cmd` alone" an assertion rather than a
/// hope, and the `./` is the unix spelling `cmd_relative` normalises.
const CMD: &str = "./bin/demo-server";

/// A temporary install root, asserted to be under the tempdir before anything writes to it.
fn rooted(tmp: &tempfile::TempDir) -> PathBuf {
    let root = tmp.path().join("agents");
    assert!(
        root.starts_with(tmp.path()),
        "every T5 case writes under the tempdir and nowhere else"
    );
    root
}

/// One entry of a fixture archive: the path inside it, its bytes, and the mode it declares.
struct Entry {
    name: &'static str,
    body: &'static [u8],
    /// The unix mode the archive itself carries. `0o644` on the `cmd` is the case hazard H-5
    /// exists for: `launch::spawn` runs `which` even on an absolute path, and `which` refuses a
    /// file nobody may execute.
    mode: u32,
}

/// The tree both fixture archives carry: the `cmd`, unexecutable in the archive, and a sibling
/// file that is not the `cmd` at all.
fn tree() -> Vec<Entry> {
    vec![
        Entry {
            name: "bin/demo-server",
            body: b"#!/bin/sh\nexit 0\n",
            mode: 0o644,
        },
        Entry {
            name: "share/notice.txt",
            body: b"the rest of the tree\n",
            mode: 0o755,
        },
    ]
}

/// `tree()` as a deflate `.zip`, in memory.
fn zip_bytes(entries: &[Entry]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for entry in entries {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(entry.mode);
        writer
            .start_file(entry.name, options)
            .expect("the entry starts");
        std::io::Write::write_all(&mut writer, entry.body).expect("the entry is written");
    }
    writer.finish().expect("the archive closes").into_inner()
}

/// `tree()` as a gzipped `.tar`, in memory.
fn tar_gz_bytes(entries: &[Entry]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    for entry in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(entry.body.len() as u64);
        header.set_mode(entry.mode);
        header.set_cksum();
        builder
            .append_data(&mut header, entry.name, entry.body)
            .expect("the entry is appended");
    }
    builder
        .into_inner()
        .expect("the tar closes")
        .finish()
        .expect("the gzip closes")
}

/// The same, with the entry names written **raw** into the header.
///
/// `tar::Builder::append_data` refuses a path containing `..` outright — which is a decent crate
/// being decent, and exactly why it cannot be the installer's guard: the archives an installer
/// reads are written by somebody else. This helper writes the bytes a hostile archive would
/// carry, so the refusal under test is `install/archive.rs`'s own and not `tar`'s.
fn hostile_tar_gz_bytes(entries: &[Entry]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    for entry in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(entry.body.len() as u64);
        header.set_mode(entry.mode);
        header.set_entry_type(tar::EntryType::Regular);
        let raw = &mut header.as_old_mut().name;
        raw.fill(0);
        raw[..entry.name.len()].copy_from_slice(entry.name.as_bytes());
        header.set_cksum();
        builder
            .append(&header, entry.body)
            .expect("the raw entry is appended");
    }
    builder
        .into_inner()
        .expect("the tar closes")
        .finish()
        .expect("the gzip closes")
}

/// One entry in the shapes an archive can carry beyond a plain file.
///
/// [`Entry`] says what an entry *contains*; this says what it **is** — a directory, a symlink, or
/// the metadata header `git archive` writes. Every name and every link target goes into the header
/// raw, for [`hostile_tar_gz_bytes`]'s reason: the archives an installer reads are written by
/// somebody else, so a fixture that let `tar` normalise a path first would be asserting `tar`'s
/// guard rather than `install/archive.rs`'s.
enum Item {
    /// A regular file with the mode the archive declares.
    File {
        name: &'static str,
        body: &'static [u8],
        mode: u32,
    },
    /// A directory entry.
    Dir { name: &'static str },
    /// A symlink at `name` pointing at `target`.
    Link {
        name: &'static str,
        target: &'static str,
    },
    /// The `pax_global_header` `git archive` puts at the head of every tarball it writes. It
    /// carries repository metadata and no file, and it has no counterpart in a zip.
    PaxGlobal { body: &'static [u8] },
}

/// `items` as a `.zip`. The `PaxGlobal` shape has no zip spelling and is skipped.
fn zip_items(items: &[Item]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let stored = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Stored)
        .unix_permissions(0o755);
    for item in items {
        match item {
            Item::File { name, body, mode } => {
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated)
                    .unix_permissions(*mode);
                writer.start_file(*name, options).expect("the entry starts");
                std::io::Write::write_all(&mut writer, body).expect("the entry is written");
            }
            Item::Dir { name } => {
                writer
                    .add_directory(*name, stored)
                    .expect("the directory entry");
            }
            Item::Link { name, target } => {
                writer
                    .add_symlink(*name, *target, stored)
                    .expect("the symlink entry");
            }
            Item::PaxGlobal { .. } => {}
        }
    }
    writer.finish().expect("the archive closes").into_inner()
}

/// `items` as a gzipped `.tar`, every header field written by hand.
fn tar_gz_items(items: &[Item]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);
    for item in items {
        let mut header = tar::Header::new_gnu();
        let (name, body): (&str, &[u8]) = match item {
            Item::File { name, body, mode } => {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(*mode);
                (name, body)
            }
            Item::Dir { name } => {
                header.set_entry_type(tar::EntryType::Directory);
                header.set_mode(0o755);
                (name, b"")
            }
            Item::Link { name, target } => {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_mode(0o777);
                let raw = &mut header.as_old_mut().linkname;
                raw.fill(0);
                raw[..target.len()].copy_from_slice(target.as_bytes());
                (name, b"")
            }
            Item::PaxGlobal { body } => {
                header.set_entry_type(tar::EntryType::XGlobalHeader);
                header.set_mode(0o644);
                ("pax_global_header", body)
            }
        };
        header.set_size(body.len() as u64);
        let raw = &mut header.as_old_mut().name;
        raw.fill(0);
        raw[..name.len()].copy_from_slice(name.as_bytes());
        header.set_cksum();
        builder
            .append(&header, body)
            .expect("the raw entry is appended");
    }
    builder
        .into_inner()
        .expect("the tar closes")
        .finish()
        .expect("the gzip closes")
}

/// `unpack` over `items` in both shapes, into a fresh directory named after the shape.
///
/// Answers `(the shape, the directory it unpacked into, what it answered)`, so a case can make the
/// same assertion twice without writing the loop twice.
async fn unpack_both(
    tmp: &Path,
    case: &str,
    items: &[Item],
) -> Vec<(&'static str, PathBuf, Result<(), InstallError>)> {
    let cancel = CancellationToken::new();
    let written = std::sync::atomic::AtomicU64::new(0);
    let mut answers = Vec::new();
    for (shape, format, bytes) in [
        ("zip", ArchiveFormat::Zip, zip_items(items)),
        ("tar", ArchiveFormat::TarGz, tar_gz_items(items)),
    ] {
        let archive = archive_file(tmp, &format!("{case}-{shape}.bin"), &bytes).await;
        // A directory of its own per shape, so "nothing outside the tree" is asserted about a
        // parent this case owns rather than about the tempdir every other case shares.
        let outside = tmp.join(format!("{case}-{shape}"));
        let into = outside.join("into");
        std::fs::create_dir_all(&outside).expect("the enclosing directory");
        let answer =
            htui_agent::install::archive::unpack(&archive, &into, format, &cancel, &written);
        answers.push((shape, outside, answer));
    }
    answers
}

// ---------------------------------------------------------------------------------------------
// T5: the layout — staging, the set-aside, promotion and the sweep
// ---------------------------------------------------------------------------------------------

/// Plan D16: a same-version re-install moves the working directory out of the glob's reach
/// **before** the rename, so the promote never lands on top of something.
///
/// Both halves matter. Moving it aside is what makes the promote a plain `rename` on every
/// platform (hazard H-20: Windows refuses a rename onto an existing directory outright), and
/// keeping the old tree afterwards is what plan D3's "a failed install never costs the working
/// one" is made of — the rollback of D16(b) has something to put back.
#[tokio::test]
async fn a_same_version_reinstall_sets_the_old_directory_aside_before_promoting() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let layout = Layout::new(rooted(&tmp));

    let working = layout.version_dir(ID, VERSION);
    tokio::fs::create_dir_all(&working)
        .await
        .expect("the working version exists");
    tokio::fs::write(working.join("which"), b"old")
        .await
        .expect("the marker is written");

    let nonce = nonce();
    let staged = layout.staging_tree(ID, VERSION, &nonce);
    tokio::fs::create_dir_all(&staged)
        .await
        .expect("the staged tree exists");
    tokio::fs::write(staged.join("which"), b"new")
        .await
        .expect("the marker is written");

    let aside = layout
        .set_aside(ID, VERSION, &nonce)
        .await
        .expect("the set-aside succeeds")
        .expect("there was a version to set aside");

    assert_eq!(aside, layout.staging_previous(ID, VERSION, &nonce));
    assert!(
        !working.exists(),
        "the old tree is out of the glob's reach before the new one lands"
    );
    assert_eq!(
        tokio::fs::read_to_string(aside.join("which"))
            .await
            .unwrap(),
        "old"
    );

    let promoted = layout
        .promote(&staged, ID, VERSION)
        .await
        .expect("the promote succeeds");

    assert_eq!(promoted, working);
    assert_eq!(
        tokio::fs::read_to_string(promoted.join("which"))
            .await
            .unwrap(),
        "new"
    );
    assert!(
        aside.exists(),
        "the previous tree survives the promote: D16(b) has to be able to put it back"
    );
}

/// Hazard H-20: `promote` asserts the target is absent and names it when it is not.
///
/// On unix a `rename` onto a non-empty directory fails with `ENOTEMPTY` and onto an empty one
/// silently replaces it; on Windows it fails outright. One check in front of the call is what
/// makes the three behave the same and the message say which directory is in the way.
#[tokio::test]
async fn a_promote_onto_an_occupied_target_is_refused_by_name() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let layout = Layout::new(rooted(&tmp));
    let nonce = nonce();
    let staged = layout.staging_tree(ID, VERSION, &nonce);
    tokio::fs::create_dir_all(&staged).await.expect("staged");
    tokio::fs::create_dir_all(layout.version_dir(ID, VERSION))
        .await
        .expect("the target is occupied");

    let error = layout
        .promote(&staged, ID, VERSION)
        .await
        .expect_err("set_aside runs first, so an occupied target is a bug, not a case");

    assert!(
        error
            .to_string()
            .contains(&layout.version_dir(ID, VERSION).display().to_string()),
        "the message has to name the directory in the way: {error}"
    );
    assert!(
        staged.exists(),
        "the staged tree is untouched by the refusal"
    );
}

/// Hazard H-7 / blueprint P-10: the sweep **restores** a set-aside version whose own directory is
/// gone, rather than deleting it.
///
/// This is the failure the `.previous` suffix exists for. `AgentRuntime::shutdown` aborts the
/// install task, and an abort between the set-aside and the promote leaves the *working* version
/// sitting in `.staging/`. A sweep that only deleted by age would then destroy the very tree plan
/// D3 promises a failed install never costs. So: target absent means the promote never happened
/// and the tree comes back — immediately, whatever its age — and target present means the promote
/// did happen and the copy is dead weight the age rule may collect.
#[tokio::test]
async fn the_sweep_restores_a_set_aside_version_an_abort_left_behind() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let layout = Layout::new(rooted(&tmp));

    let orphan_nonce = nonce();
    let orphan = layout.staging_previous(ID, VERSION, &orphan_nonce);
    tokio::fs::create_dir_all(&orphan)
        .await
        .expect("the orphan");
    tokio::fs::write(orphan.join("which"), b"working")
        .await
        .expect("the marker is written");
    // The agent directory exists — the abort took the version out of it, not the whole id.
    tokio::fs::create_dir_all(layout.agent_dir(ID))
        .await
        .expect("the agent directory");

    let report = layout
        .sweep(STAGING_MAX_AGE, SystemTime::now())
        .await
        .expect("the sweep succeeds");

    assert_eq!(report.restored, vec![layout.version_dir(ID, VERSION)]);
    assert!(report.removed.is_empty(), "{report:?}");
    assert_eq!(
        tokio::fs::read_to_string(layout.version_dir(ID, VERSION).join("which"))
            .await
            .expect("the working version is back"),
        "working",
        "an aborted install must cost nothing, so the tree it set aside comes back whole"
    );
    assert!(!orphan.exists());

    // The other half: a `.previous` whose version directory is *there* was superseded by a
    // promote that succeeded, so once it is old it is residue like any other.
    let done_nonce = nonce();
    let done = layout.staging_previous(ID, VERSION, &done_nonce);
    tokio::fs::create_dir_all(&done).await.expect("the copy");
    let later = SystemTime::now() + Duration::from_secs(2 * 60 * 60);
    let report = layout
        .sweep(STAGING_MAX_AGE, later)
        .await
        .expect("the sweep succeeds");
    assert_eq!(report.removed, vec![done.clone()]);
    assert!(!done.exists());
    assert!(
        layout.version_dir(ID, VERSION).exists(),
        "and the promoted version it was a copy of is not touched"
    );
}

/// Plan D16: every install begins by sweeping `.staging/` entries older than an hour — and only
/// those. A download that is running in another process is minutes old, not hours.
#[tokio::test]
async fn the_sweep_removes_residue_older_than_an_hour_and_nothing_younger() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let layout = Layout::new(rooted(&tmp));
    let nonce = nonce();
    let archive = layout.staging_archive(ID, VERSION, &nonce);
    let tree = layout.staging_tree(ID, VERSION, &nonce);
    tokio::fs::create_dir_all(&tree).await.expect("the tree");
    tokio::fs::write(&archive, b"partial")
        .await
        .expect("the partial download");

    let report = layout
        .sweep(STAGING_MAX_AGE, SystemTime::now())
        .await
        .expect("the sweep succeeds");
    assert_eq!(
        report,
        htui_agent::install::layout::SweepReport::default(),
        "a running install's own staging entries are minutes old; sweeping them would be the \
         installer deleting its own work"
    );
    assert!(archive.exists() && tree.exists());

    let later = SystemTime::now() + Duration::from_secs(2 * 60 * 60);
    let mut report = layout.sweep(STAGING_MAX_AGE, later).await.expect("swept");
    report.removed.sort();
    let mut expected = vec![archive.clone(), tree.clone()];
    expected.sort();
    assert_eq!(report.removed, expected);
    assert!(!archive.exists() && !tree.exists());
    assert!(
        layout.staging().exists(),
        "the staging directory itself stays: the next install writes into it"
    );
}

/// The version directories under one id, with `manifest.json` — a file, not a version — excluded.
///
/// An absent root answers "none" rather than failing, because the pre-flight of the first-ever
/// install on a box runs before anything has created it.
#[tokio::test]
async fn existing_versions_lists_directories_and_ignores_the_manifest() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let layout = Layout::new(rooted(&tmp));
    assert!(layout.existing_versions(ID).await.is_empty());

    for version in ["1.1.0", "1.1.1"] {
        tokio::fs::create_dir_all(layout.version_dir(ID, version))
            .await
            .expect("a version directory");
    }
    tokio::fs::write(layout.manifest(ID), b"{}")
        .await
        .expect("the manifest");

    assert_eq!(
        layout.existing_versions(ID).await,
        vec!["1.1.0".to_owned(), "1.1.1".to_owned()]
    );

    let removed = layout
        .retain_only(ID, "1.1.1")
        .await
        .expect("the retention succeeds");
    assert_eq!(removed, vec!["1.1.0".to_owned()]);
    assert_eq!(layout.existing_versions(ID).await, vec!["1.1.1".to_owned()]);
    assert!(
        layout.manifest(ID).exists(),
        "retention deletes versions, never the record of them (plan D17)"
    );
}

// ---------------------------------------------------------------------------------------------
// T5: the manifest
// ---------------------------------------------------------------------------------------------

/// Plan D17: the manifest is written the way `identity::store` writes `box.toml` — into a
/// uniquely named temporary and then renamed over the target.
///
/// The property under test is that a reader never sees a half-written file: the directory holds
/// `manifest.json` and nothing else when the write returns, whatever was there before.
#[tokio::test]
async fn the_manifest_is_written_through_a_temporary_and_a_rename() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let layout = Layout::new(rooted(&tmp));
    let path = layout.manifest(ID);

    assert_eq!(
        Manifest::load(&path).await,
        Manifest::default(),
        "an absent manifest is an empty one: the first install must not need a file to exist"
    );

    let mut manifest = Manifest::default();
    manifest.installs.insert(
        VERSION.to_owned(),
        InstallRecord {
            sha256: "0".repeat(64),
            published: false,
            archive: "http://127.0.0.1/archive.zip".to_owned(),
            platform: "linux-x86_64".to_owned(),
            installed_at: Utc::now(),
        },
    );
    manifest
        .store(&path)
        .await
        .expect("the manifest is written under a root that did not exist yet");

    assert_eq!(
        Manifest::load(&path).await,
        manifest,
        "what was written is what is read back"
    );
    let mut names = Vec::new();
    let mut entries = tokio::fs::read_dir(layout.agent_dir(ID))
        .await
        .expect("the id directory");
    while let Some(entry) = entries.next_entry().await.expect("an entry") {
        names.push(entry.file_name().to_string_lossy().into_owned());
    }
    assert_eq!(
        names,
        vec!["manifest.json".to_owned()],
        "a temporary left behind is a file the next reader has to know to ignore: {names:?}"
    );

    // And a second write replaces it whole rather than appending to it.
    manifest.consent = Some(Consent {
        license: Some("proprietary".to_owned()),
        license_url: Some("https://example.invalid/terms".to_owned()),
        accepted_at: Utc::now(),
        version: VERSION.to_owned(),
    });
    manifest.store(&path).await.expect("the second write");
    assert_eq!(Manifest::load(&path).await, manifest);
}

/// Plan D17: consent covers one `(license, license_url)` pair and no other.
///
/// Both halves have to match. A licence that stayed `proprietary` while the terms it points at
/// moved is *new terms*, and asking again is the cheap side of that mistake — the expensive side
/// is a box that accepted one document and installed under another.
#[test]
fn consent_stops_covering_the_terms_when_the_license_url_moves() {
    let mut manifest = Manifest::default();
    assert!(
        !manifest.consent_covers(Some("proprietary"), Some("https://example.invalid/terms")),
        "nothing recorded covers nothing"
    );

    manifest.consent = Some(Consent {
        license: Some("proprietary".to_owned()),
        license_url: Some("https://example.invalid/terms".to_owned()),
        accepted_at: Utc::now(),
        version: VERSION.to_owned(),
    });

    assert!(manifest.consent_covers(Some("proprietary"), Some("https://example.invalid/terms")));
    assert!(
        !manifest.consent_covers(
            Some("proprietary"),
            Some("https://example.invalid/terms-v2")
        ),
        "the same licence id at a different URL is a different document"
    );
    assert!(
        !manifest.consent_covers(Some("Apache-2.0"), Some("https://example.invalid/terms")),
        "and a different licence id at the same URL is a different licence"
    );
    assert!(!manifest.consent_covers(None, None));
}

/// A hand-edited manifest that does not parse must not block an install: the worst an empty one
/// costs is asking for consent that was already given once.
#[tokio::test]
async fn a_manifest_that_does_not_parse_reads_as_an_empty_one() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let layout = Layout::new(rooted(&tmp));
    let path = layout.manifest(ID);
    tokio::fs::create_dir_all(layout.agent_dir(ID))
        .await
        .expect("the id directory");
    tokio::fs::write(&path, b"{ not json at all")
        .await
        .expect("the damaged manifest");

    assert_eq!(Manifest::load(&path).await, Manifest::default());
}

// ---------------------------------------------------------------------------------------------
// T5: the unpacker
// ---------------------------------------------------------------------------------------------

/// The bytes of one archive, written under `tmp` because the unpacker reads a file and never a
/// stream: a zip's directory is at its **end**, and the digest is checked over the whole file
/// before an entry of it is trusted (plan D10).
async fn archive_file(tmp: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    tokio::fs::create_dir_all(tmp).await.expect("the directory");
    let path = tmp.join(name);
    tokio::fs::write(&path, bytes)
        .await
        .expect("the archive is written");
    path
}

/// Everything under `dir`, as paths relative to it, sorted — what "the whole tree" is asserted in.
fn walk_relative(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(
                    path.strip_prefix(dir)
                        .expect("under the tree")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    found.sort();
    found
}

/// The `.zip` arm: the whole tree, the modes the archive declares, and a byte counter the progress
/// poller can read while the blocking thread runs.
#[tokio::test]
async fn a_zip_unpacks_the_whole_tree_and_counts_what_it_wrote() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let entries = tree();
    let archive = archive_file(tmp.path(), "demo.zip", &zip_bytes(&entries)).await;
    let into = tmp.path().join("unpacked");
    let written = std::sync::atomic::AtomicU64::new(0);

    htui_agent::install::archive::unpack(
        &archive,
        &into,
        ArchiveFormat::Zip,
        &CancellationToken::new(),
        &written,
    )
    .expect("the archive unpacks");

    assert_eq!(
        walk_relative(&into),
        vec!["bin/demo-server".to_owned(), "share/notice.txt".to_owned()],
        "an install is the whole tree: an adapter that ships a data file beside its binary needs \
         the data file"
    );
    assert_eq!(
        written.load(std::sync::atomic::Ordering::Relaxed),
        entries
            .iter()
            .map(|entry| entry.body.len() as u64)
            .sum::<u64>(),
        "the counter is what the progress poller reads while the blocking thread is running"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |name: &str| {
            std::fs::metadata(into.join(name))
                .expect("the entry")
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("share/notice.txt"), 0o755, "the archive said 0755");
        assert_eq!(mode("bin/demo-server"), 0o644, "and it said 0644 here");
    }
}

/// The `.tar.gz` arm over the identical tree: same files, same modes, same counter. Nothing above
/// this module was told which of the two shapes it was handling.
#[tokio::test]
async fn the_tar_gz_twin_of_the_same_tree_unpacks_identically() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let entries = tree();
    let zipped = tmp.path().join("from-zip");
    let tarred = tmp.path().join("from-tar");
    let written = std::sync::atomic::AtomicU64::new(0);
    let cancel = CancellationToken::new();

    let zip = archive_file(tmp.path(), "demo.zip", &zip_bytes(&entries)).await;
    let tar = archive_file(tmp.path(), "demo.tar.gz", &tar_gz_bytes(&entries)).await;
    htui_agent::install::archive::unpack(&zip, &zipped, ArchiveFormat::Zip, &cancel, &written)
        .expect("the zip unpacks");
    htui_agent::install::archive::unpack(&tar, &tarred, ArchiveFormat::TarGz, &cancel, &written)
        .expect("the tarball unpacks");

    assert_eq!(walk_relative(&zipped), walk_relative(&tarred));
    for name in walk_relative(&tarred) {
        assert_eq!(
            std::fs::read(tarred.join(&name)).expect("the entry"),
            std::fs::read(zipped.join(&name)).expect("the entry"),
            "{name} differs between the two shapes"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |root: &Path| {
                std::fs::metadata(root.join(&name))
                    .expect("the entry")
                    .permissions()
                    .mode()
                    & 0o777
            };
            assert_eq!(mode(&tarred), mode(&zipped), "{name}'s mode differs");
        }
    }
}

/// Hazard H-16: an entry that climbs out of the staging tree is refused, in **both** shapes, by
/// this crate's own check.
///
/// The check is ours and not the archive crate's on purpose. `zip`'s `enclosed_name` and `tar`'s
/// `unpack_in` each have a rule, but the two rules are not the same rule, and one of them not
/// covering a case would be a write outside the tree — the one failure an installer must not have.
/// So the entry path is split on **both** separators and refused for an absolute prefix, a drive
/// letter or a `..`, exactly as `cmd_relative` does it.
#[tokio::test]
async fn an_entry_that_climbs_out_of_the_tree_is_refused_in_both_shapes() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let cancel = CancellationToken::new();
    let written = std::sync::atomic::AtomicU64::new(0);

    for (index, name) in ["../escape", "/etc/escape"].iter().enumerate() {
        let entries = vec![Entry {
            name,
            body: b"no\n",
            mode: 0o644,
        }];
        for (shape, format, bytes) in [
            ("zip", ArchiveFormat::Zip, zip_bytes(&entries)),
            ("tar", ArchiveFormat::TarGz, hostile_tar_gz_bytes(&entries)),
        ] {
            let archive = archive_file(tmp.path(), &format!("{shape}-{index}.bin"), &bytes).await;
            let into = tmp.path().join(format!("into-{shape}-{index}"));
            let error =
                htui_agent::install::archive::unpack(&archive, &into, format, &cancel, &written)
                    .expect_err("an entry that leaves the tree is not unpacked");

            assert!(
                matches!(&error, InstallError::Archive { message } if message.contains(name)),
                "the refusal has to name the entry it refused: {error:?}"
            );
            assert!(
                !tmp.path().join("escape").exists() && !into.join("escape").exists(),
                "nothing outside the staging tree may exist afterwards"
            );
        }
    }
}

/// Blueprint P-3 / hazard H-8: `JoinHandle::abort` does not stop a `spawn_blocking` thread, so the
/// unpack stops itself — it reads the token before every entry.
///
/// Without this, shutting the app down during a 2 GB unpack would leave a thread writing into
/// `.staging/` after the runtime that owned it is gone.
#[tokio::test]
async fn an_unpack_whose_token_is_tripped_writes_nothing_more() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let archive = archive_file(tmp.path(), "demo.zip", &zip_bytes(&tree())).await;
    let into = tmp.path().join("unpacked");
    let cancel = CancellationToken::new();
    cancel.cancel();

    let error = htui_agent::install::archive::unpack(
        &archive,
        &into,
        ArchiveFormat::Zip,
        &cancel,
        &std::sync::atomic::AtomicU64::new(0),
    )
    .expect_err("a tripped token stops the unpack");

    assert!(matches!(error, InstallError::Cancelled), "{error:?}");
    assert!(
        walk_relative(&into).is_empty(),
        "the entry after the cancellation is the one that is not written"
    );
}

/// Hazard H-5: the `cmd` is made executable whatever the archive said.
///
/// `launch::spawn` resolves through `which` even for an absolute path, and `which` refuses a file
/// nobody may execute. An adapter whose tarball carries plain `0644` would otherwise probe
/// `failed` with "is not executable" — the README's `chmod +x`, made structural.
#[tokio::test]
#[cfg(unix)]
async fn the_cmd_is_made_executable_although_the_archive_said_0644() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("a temporary directory");
    let archive = archive_file(tmp.path(), "demo.zip", &zip_bytes(&tree())).await;
    let into = tmp.path().join("unpacked");
    htui_agent::install::archive::unpack(
        &archive,
        &into,
        ArchiveFormat::Zip,
        &CancellationToken::new(),
        &std::sync::atomic::AtomicU64::new(0),
    )
    .expect("the archive unpacks");

    let cmd = into.join("bin/demo-server");
    assert_eq!(
        std::fs::metadata(&cmd).unwrap().permissions().mode() & 0o111,
        0,
        "the archive really does withhold the bit"
    );

    htui_agent::install::archive::make_executable(&cmd).expect("the bit is set");

    assert_eq!(
        std::fs::metadata(&cmd).unwrap().permissions().mode() & 0o777,
        0o755,
        "read where it was readable, execute where it was readable, and nothing widened"
    );
}

/// Hazard H-16, the case a lexical check cannot see: an archive that builds its own ladder out of
/// the tree and then writes a file through it.
///
/// `d/a -> ..` is inside the tree by every rule there is. `b -> d/a/..` is *lexically* one level
/// down — one `Normal`, one `Normal`, one `ParentDir` — and is actually the tree's parent, because
/// `a` is a link and a component count cannot know that. A plain file `b/pwned` then lands outside
/// the staging tree: `into/b/pwned` starts with `into` as a string, and `create_dir_all` and
/// `File::create` both follow links.
///
/// Nothing lexical closes this. The unpacker walks every target down from `into` one component at
/// a time instead, and refuses the moment a component that already exists is a link — which is the
/// only rule under which "nothing is written through a symlink" is a fact rather than an argument
/// about which paths spell an escape.
///
/// Unix only: an unprivileged Windows process cannot create a symlink at all, so `create_link`
/// writes a small file naming the target (plan D21) and there is nothing to be written *through*.
#[cfg(unix)]
#[tokio::test]
async fn a_chained_symlink_is_never_written_through_in_either_shape() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let items = [
        Item::Dir { name: "d" },
        Item::Link {
            name: "d/a",
            target: "..",
        },
        Item::Link {
            name: "b",
            target: "d/a/..",
        },
        Item::File {
            name: "b/pwned",
            body: b"owned\n",
            mode: 0o644,
        },
    ];

    for (shape, outside, answer) in unpack_both(tmp.path(), "chain", &items).await {
        let error = answer.expect_err(&format!(
            "{shape}: the ladder out of the tree has to be refused, not followed"
        ));
        assert!(
            matches!(&error, InstallError::Archive { message } if message.contains("symlink")),
            "{shape}: the refusal has to say which link it would have been written through: \
             {error:?}"
        );
        assert!(
            !outside.join("pwned").exists(),
            "{shape}: `b/pwned` resolves to the staging tree's own parent, and an installer that \
             writes there once writes anywhere"
        );
    }
}

/// The two symlinks a lexical check does catch, in both shapes: straight out of the tree, and
/// straight at an absolute path.
///
/// Kept beside the chained case on purpose. The component count is still the first guard — it is
/// what stops a link that *points* somewhere it should not from ever being created — and the
/// component walk is the second, for the links it cannot reason about.
#[cfg(unix)]
#[tokio::test]
async fn a_symlink_that_leaves_the_tree_is_refused_in_either_shape() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    for (case, target) in [("up", "../../escape"), ("absolute", "/etc/passwd")] {
        let items = [Item::Link {
            name: "bin/escape",
            target,
        }];
        for (shape, outside, answer) in unpack_both(tmp.path(), case, &items).await {
            let error = answer.expect_err("a link out of the tree is not written");
            assert!(
                matches!(&error, InstallError::Archive { message } if message.contains(target)),
                "{shape}/{case}: the refusal has to name the target: {error:?}"
            );
            assert!(
                !outside.join("into/bin/escape").exists(),
                "{shape}/{case}: and the link itself is not left behind either"
            );
        }
    }
}

/// The other half of the rule: a relative link that stays inside is an ordinary part of an adapter
/// and is written as a link.
///
/// Several published adapters ship one — a versioned `.so` beside its unversioned name — and a
/// guard that refused them all would trade one bug for a different one.
#[cfg(unix)]
#[tokio::test]
async fn an_in_tree_relative_symlink_is_written_in_either_shape() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let items = [
        Item::File {
            name: "bin/demo-server",
            body: b"#!/bin/sh\nexit 0\n",
            mode: 0o755,
        },
        Item::Link {
            name: "bin/demo",
            target: "demo-server",
        },
    ];

    for (shape, outside, answer) in unpack_both(tmp.path(), "inside", &items).await {
        answer.unwrap_or_else(|error| {
            panic!("{shape}: an adapter's own link is not a hazard: {error:?}")
        });
        let link = outside.join("into/bin/demo");
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("the link is there")
                .is_symlink(),
            "{shape}: it has to be a link and not a copy"
        );
        assert_eq!(
            std::fs::read_link(&link).expect("the target"),
            Path::new("demo-server"),
            "{shape}: pointing where the archive said"
        );
        assert_eq!(
            std::fs::read(&link).expect("through the link"),
            b"#!/bin/sh\nexit 0\n",
            "{shape}: and resolving to the file beside it"
        );
    }
}

/// `tar -C dir -czf x.tgz .` — the most common way a tarball is made — writes `./`, `./bin/` and
/// `./bin/demo` as its entries, and every one of them has to unpack.
///
/// `./` names the staging tree itself, which as a *directory* is already there and is nothing to
/// do. Refusing it as "names nothing" rejected a real adapter on its first entry.
#[tokio::test]
async fn a_dot_prefixed_archive_unpacks_in_either_shape() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let items = [
        Item::Dir { name: "./" },
        Item::Dir { name: "./bin" },
        Item::File {
            name: "./bin/demo-server",
            body: b"#!/bin/sh\nexit 0\n",
            mode: 0o755,
        },
        Item::File {
            name: "./share/notice.txt",
            body: b"the rest of the tree\n",
            mode: 0o644,
        },
    ];

    for (shape, outside, answer) in unpack_both(tmp.path(), "dot", &items).await {
        answer
            .unwrap_or_else(|error| panic!("{shape}: a `./`-prefixed archive unpacks: {error:?}"));
        assert_eq!(
            walk_relative(&outside.join("into")),
            vec!["bin/demo-server".to_owned(), "share/notice.txt".to_owned()],
            "{shape}: the `./` is a prefix and not a directory of its own"
        );
    }
}

/// The other side of that: `./` as a **file** names the tree itself and is refused.
///
/// Tar only, because a zip says "directory" with a trailing slash and there is no way to spell a
/// regular file called `./` in one. This is what keeps the leniency above from letting an archive
/// have the unpacker create — or chmod — its own staging root as though it were a file.
#[tokio::test]
async fn a_file_entry_that_names_the_tree_itself_is_refused() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let bytes = tar_gz_items(&[Item::File {
        name: "./",
        body: b"not a tree\n",
        mode: 0o644,
    }]);
    let archive = archive_file(tmp.path(), "self.tar.gz", &bytes).await;
    let into = tmp.path().join("into-self");

    let error = htui_agent::install::archive::unpack(
        &archive,
        &into,
        ArchiveFormat::TarGz,
        &CancellationToken::new(),
        &std::sync::atomic::AtomicU64::new(0),
    )
    .expect_err("a file that is the tree is not a file");

    assert!(
        matches!(&error, InstallError::Archive { message } if message.contains("the tree itself")),
        "{error:?}"
    );
}

/// A `git archive` tarball opens with a `pax_global_header`. It carries repository metadata and no
/// file; `tar`'s own `unpack` skips it, and so does this one.
///
/// Refusing it as "an entry of a kind this installer does not write" rejected every tarball built
/// that way — which is how a good part of the registry publishes.
#[tokio::test]
async fn a_pax_global_header_is_skipped_and_the_rest_of_the_tarball_unpacks() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let bytes = tar_gz_items(&[
        Item::PaxGlobal {
            body: b"52 comment=0000000000000000000000000000000000000000\n",
        },
        Item::File {
            name: "bin/demo-server",
            body: b"#!/bin/sh\nexit 0\n",
            mode: 0o755,
        },
    ]);
    let archive = archive_file(tmp.path(), "git.tar.gz", &bytes).await;
    let into = tmp.path().join("into-git");

    htui_agent::install::archive::unpack(
        &archive,
        &into,
        ArchiveFormat::TarGz,
        &CancellationToken::new(),
        &std::sync::atomic::AtomicU64::new(0),
    )
    .expect("a `git archive` tarball is an ordinary tarball");

    assert_eq!(walk_relative(&into), vec!["bin/demo-server".to_owned()]);
}

/// A zip stores a symlink's target as the entry's **body**, so reading it is reading a length the
/// archive chose. It is capped, and an entry that exceeds the cap is refused as an archive rather
/// than surfacing as whatever `symlink(2)` happens to say about it.
#[tokio::test]
async fn a_zip_symlink_whose_target_is_enormous_is_refused_as_an_archive() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .add_symlink(
            "bin/demo",
            "x".repeat(8192),
            zip::write::SimpleFileOptions::default(),
        )
        .expect("the symlink entry");
    let bytes = writer.finish().expect("the archive closes").into_inner();
    let archive = archive_file(tmp.path(), "long.zip", &bytes).await;
    let into = tmp.path().join("into-long");

    let error = htui_agent::install::archive::unpack(
        &archive,
        &into,
        ArchiveFormat::Zip,
        &CancellationToken::new(),
        &std::sync::atomic::AtomicU64::new(0),
    )
    .expect_err("a target no filesystem would store is refused");

    assert!(
        matches!(&error, InstallError::Archive { message } if message.contains("longer than")),
        "an unbounded read is the bug; the refusal is the fix: {error:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// T5: the fetch, and the sequence an install makes of these pieces
// ---------------------------------------------------------------------------------------------

/// A one-entry registry document, built here rather than pinned: these cases are about the bytes
/// an archive URL serves, and the entry exists only to carry that URL, a `cmd` and a digest.
fn one_entry_registry(archive: &str, cmd: &str, sha256: Option<&str>) -> Value {
    let mut binary = json!({ "archive": archive, "cmd": cmd, "args": [], "env": {} });
    if let Some(digest) = sha256 {
        binary["sha256"] = json!(digest);
    }
    json!({
        "version": "1.0.0",
        "agents": [{
            "id": ID,
            "name": "Demo",
            "version": VERSION,
            "license": "Apache-2.0",
            "license_url": "https://example.invalid/terms",
            "distribution": { "binary": { "linux-x86_64": binary } },
        }],
    })
}

/// The glob the row declares and the installer has to satisfy: the seed shape, with the install
/// root as a token and the version as the `*`.
fn demo_glob() -> Value {
    json!({
        "kind": "glob",
        "patterns": [],
        "platform": {
            "linux-x86_64": {
                "patterns": [format!("%HTUI_AGENTS_ROOT%/{ID}/*/bin/demo-server")],
            },
        },
    })
}

/// A row pointing at that entry through the tool that glob is declared under.
fn demo_row() -> Agent {
    row(
        "demo",
        json!({ "demo_server": demo_glob() }),
        Some(json!({ "source": "acp_registry", "id": ID, "tool": "demo_server" })),
    )
}

/// A fixture server serving one archive in full, and the plan that names it.
struct Fetched {
    _tmp: tempfile::TempDir,
    installer: Installer,
    plan: htui_agent::InstallPlan,
    layout: Layout,
    env: ProbeEnv,
}

impl Fetched {
    /// The rig: `path` serves `body`, the entry publishes `sha256`, and the pre-flight has run.
    async fn new(path: &str, body: Vec<u8>, sha256: Option<&str>) -> Self {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let fixture = Fixture::start().await;
        let document = one_entry_registry(&fixture.url(path), CMD, sha256);
        fixture.route("/registry.json", registry_route(&document, 300, "\"v1\""));
        fixture.route(
            path,
            Route {
                status: 200,
                body,
                ..Route::default()
            },
        );
        let config = InstallConfig::new(fixture.base(), Some(rooted(&tmp)));
        let installer = Installer::new(config.clone()).expect("a client builds");
        let env = boxed(&config, tmp.path(), "linux-x86_64");
        let plan = plan(&installer, &demo_row(), &env, Utc::now())
            .await
            .expect("the pre-flight succeeds");
        let layout = Layout::new(plan.root.clone());
        Self {
            _tmp: tmp,
            installer,
            plan,
            layout,
            env,
        }
    }

    /// The staging archive path this rig's install would use.
    fn archive_at(&self, nonce: &str) -> PathBuf {
        self.layout
            .staging_archive(&self.plan.registry_id, &self.plan.version, nonce)
    }

    /// The staging tree path this rig's install would use.
    fn tree_at(&self, nonce: &str) -> PathBuf {
        self.layout
            .staging_tree(&self.plan.registry_id, &self.plan.version, nonce)
    }

    /// The download, with every frame recorded.
    async fn download(
        &self,
        into: &Path,
    ) -> (
        Result<htui_agent::install::fetch::Downloaded, InstallError>,
        Vec<InstallProgress>,
    ) {
        let mut frames = Vec::new();
        let outcome = htui_agent::install::fetch::download(
            &self.installer,
            &self.plan,
            into,
            &mut Throttle::new(Duration::ZERO),
            &mut |frame| frames.push(frame),
            &CancellationToken::new(),
        )
        .await;
        (outcome, frames)
    }
}

/// Lowercase hex sha256, the spelling `identity::db_fingerprint` uses and the one the manifest
/// records.
fn hex_digest(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The archive streams into `.staging/<id>-<version>-<nonce>.archive`, the progress adds up to the
/// size the `HEAD` declared, and the digest is `sha2`'s over the same bytes.
///
/// Where it lands is half the point (hazard H-3): `.staging/` is a sibling of `<id>/`, so a body
/// that stops half way is a file no glob can reach, and the file name carries a nonce so two runs
/// can never write the same one.
#[tokio::test]
async fn an_archive_streams_into_staging_with_progress_and_a_digest_over_the_same_bytes() {
    let body = zip_bytes(&tree());
    let rig = Fetched::new("/archive.zip", body.clone(), None).await;
    let into = rig.archive_at(&nonce());

    let (downloaded, frames) = rig.download(&into).await;
    let downloaded = downloaded.expect("the archive downloads");

    assert_eq!(downloaded.path, into);
    assert!(
        into.starts_with(rig.layout.staging()),
        "a body in flight lives where no glob walks"
    );
    assert_eq!(downloaded.bytes, body.len() as u64);
    assert_eq!(
        tokio::fs::read(&into).await.expect("the file is there"),
        body,
        "what was written is what was served"
    );
    assert_eq!(
        downloaded.sha256,
        hex_digest(&body),
        "the digest is computed as the bytes go past, not by reading the file back"
    );

    assert!(!frames.is_empty(), "a download reports its progress");
    assert!(
        frames
            .iter()
            .all(|frame| frame.phase == InstallPhase::Downloading),
        "{frames:?}"
    );
    assert!(
        frames.windows(2).all(|pair| pair[1].done >= pair[0].done),
        "a progress bar that goes backwards is a bug report: {frames:?}"
    );
    let last = frames.last().expect("a final frame");
    assert_eq!(last.done, body.len() as u64);
    assert_eq!(
        last.total,
        Some(body.len() as u64),
        "the denominator is the `content-length` the pre-flight already showed the user"
    );
    assert_eq!(
        rig.plan.content_length,
        Some(body.len() as u64),
        "and it is the same number the consent pane said"
    );
}

/// Plan D2: a published digest that does not match stops the install **before** anything is
/// unpacked.
///
/// The archive stays on disk for the sweep to collect; what must not exist is a tree. Verifying
/// after unpacking would mean deleting an attacker's files rather than never writing them.
#[tokio::test]
async fn a_published_digest_that_does_not_match_stops_before_anything_is_unpacked() {
    let body = zip_bytes(&tree());
    let wrong = "f".repeat(64);
    let rig = Fetched::new("/archive.zip", body.clone(), Some(&wrong)).await;
    let nonce = nonce();
    let into = rig.archive_at(&nonce);

    let (downloaded, _) = rig.download(&into).await;
    let downloaded = downloaded.expect("the archive downloads");

    let error =
        htui_agent::install::fetch::verify_digest(rig.plan.sha256.as_deref(), &downloaded.sha256)
            .expect_err("the published digest is not this archive's");

    assert!(
        matches!(&error, InstallError::DigestMismatch { expected, computed }
            if expected.as_str() == wrong && computed == &downloaded.sha256),
        "unexpected refusal: {error:?}"
    );
    assert!(
        error.to_string().contains("nothing was unpacked"),
        "the user is owed the fact that no file was written: {error}"
    );
    assert!(
        !rig.tree_at(&nonce).exists(),
        "and no file was written: the verify runs between the download and the unpack"
    );
}

/// Plan D2 and D17: eight of the registry's entries publish no digest, so `htui` records the one
/// it computed and says out loud that it is not a verification.
#[tokio::test]
async fn an_entry_with_no_published_digest_records_what_was_received() {
    let body = zip_bytes(&tree());
    let rig = Fetched::new("/archive.zip", body.clone(), None).await;
    let into = rig.archive_at(&nonce());
    let (downloaded, _) = rig.download(&into).await;
    let downloaded = downloaded.expect("the archive downloads");

    let published =
        htui_agent::install::fetch::verify_digest(rig.plan.sha256.as_deref(), &downloaded.sha256)
            .expect("an unpublished digest cannot mismatch");
    assert!(!published);

    let mut manifest = Manifest::default();
    manifest.installs.insert(
        rig.plan.version.clone(),
        InstallRecord {
            sha256: downloaded.sha256.clone(),
            published,
            archive: rig.plan.archive_url.clone(),
            platform: rig.plan.platform.clone(),
            installed_at: Utc::now(),
        },
    );
    let path = rig.layout.manifest(&rig.plan.registry_id);
    manifest
        .store(&path)
        .await
        .expect("the manifest is written");

    let record = Manifest::load(&path)
        .await
        .installs
        .remove(VERSION)
        .expect("the version is recorded");
    assert_eq!(record.sha256, hex_digest(&body));
    assert!(
        !record.published,
        "recording a computed digest as published would turn `htui cannot verify this` into a \
         claim that it did"
    );
}

/// A cancelled download leaves no partial file behind (hazard H-6).
///
/// A 400 MB `.archive` sitting in staging until the next sweep is exactly the residue `x` is
/// pressed to avoid.
#[tokio::test]
async fn a_cancelled_download_removes_its_own_partial_file() {
    let rig = Fetched::new("/archive.zip", zip_bytes(&tree()), None).await;
    let into = rig.archive_at(&nonce());
    let cancel = CancellationToken::new();
    cancel.cancel();

    let error = htui_agent::install::fetch::download(
        &rig.installer,
        &rig.plan,
        &into,
        &mut Throttle::new(Duration::ZERO),
        &mut |_| {},
        &cancel,
    )
    .await
    .expect_err("a tripped token stops the download");

    assert!(matches!(error, InstallError::Cancelled), "{error:?}");
    assert!(!into.exists(), "the partial file goes with it");
}

/// The sequence T6 will wrap in one call, run here by hand: download, verify, unpack, make the
/// `cmd` executable **in staging**, promote.
///
/// Three properties at once. The promoted directory holds the **whole** tree, because an adapter
/// that ships data beside its binary needs the data. The `cmd` is executable although the archive
/// said `0644` (hazard H-5), and it was made so before the rename, so the promoted tree is never
/// unrunnable for an instant. And the row's own glob — the same one the probe uses — now resolves
/// exactly the file that was written, which is plan D5's agreement and what T6's post-promote
/// check will assert against.
#[tokio::test]
async fn the_promoted_tree_is_whole_executable_and_where_the_row_looks() {
    let body = zip_bytes(&tree());
    let rig = Fetched::new("/archive.zip", body.clone(), Some(&hex_digest(&body))).await;
    let nonce = nonce();

    let (downloaded, _) = rig.download(&rig.archive_at(&nonce)).await;
    let downloaded = downloaded.expect("the archive downloads");
    assert!(
        htui_agent::install::fetch::verify_digest(rig.plan.sha256.as_deref(), &downloaded.sha256)
            .expect("the digests agree"),
        "this entry publishes one, so the record says so"
    );

    let tree = rig.tree_at(&nonce);
    htui_agent::install::archive::unpack(
        &downloaded.path,
        &tree,
        rig.plan.format,
        &CancellationToken::new(),
        &std::sync::atomic::AtomicU64::new(0),
    )
    .expect("the archive unpacks");
    htui_agent::install::archive::make_executable(&tree.join("bin/demo-server"))
        .expect("the cmd is made executable in staging");

    let promoted = rig
        .layout
        .promote(&tree, &rig.plan.registry_id, &rig.plan.version)
        .await
        .expect("the promote succeeds");

    assert_eq!(promoted, rig.plan.install_dir);
    assert_eq!(
        walk_relative(&promoted),
        vec!["bin/demo-server".to_owned(), "share/notice.txt".to_owned()],
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(promoted.join("bin/demo-server"))
                .expect("the cmd")
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "`which` refuses a 0644 file even by absolute path, so the bit is not optional"
        );
    }

    let resolved = resolve_tool(
        &serde_json::from_value(demo_glob()).expect("the glob parses"),
        &rig.env,
    )
    .await
    .expect("the resolver runs")
    .expect("the row's glob finds what was installed");
    assert_eq!(resolved.path, promoted.join("bin/demo-server"));
}

/// Hazard H-3, stated as the failure it prevents: an install killed between the unpack and the
/// promote leaves residue in `.staging/` and **nothing the row's glob can resolve**.
///
/// This is the metric the PRD writes as "an interrupted install leaves nothing behind that the
/// probe would resolve". It holds by layout rather than by cleanup: `.staging/` is a sibling of
/// `<id>/`, and the seed pattern's literal root ends at `<id>`, so a half-finished tree is not
/// merely unlikely to be walked — it is outside the walk.
#[tokio::test]
async fn a_kill_between_unpack_and_promote_leaves_nothing_the_glob_resolves() {
    let rig = Fetched::new("/archive.zip", zip_bytes(&tree()), None).await;
    let nonce = nonce();
    let (downloaded, _) = rig.download(&rig.archive_at(&nonce)).await;
    let downloaded = downloaded.expect("the archive downloads");
    let tree = rig.tree_at(&nonce);
    htui_agent::install::archive::unpack(
        &downloaded.path,
        &tree,
        rig.plan.format,
        &CancellationToken::new(),
        &std::sync::atomic::AtomicU64::new(0),
    )
    .expect("the archive unpacks");

    // The kill: the future that would have promoted is dropped before it reaches the rename. A
    // zero timeout is the drop — `AgentRuntime::shutdown`'s `abort()` is the same thing at the
    // same place.
    let killed = tokio::time::timeout(Duration::ZERO, async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        rig.layout
            .promote(&tree, &rig.plan.registry_id, &rig.plan.version)
            .await
    })
    .await;
    assert!(killed.is_err(), "the promote never ran");

    assert!(
        !rig.plan.install_dir.exists(),
        "the version directory was never created"
    );
    assert!(
        tree.exists(),
        "the residue is in staging, for the next sweep"
    );
    assert!(
        downloaded.path.exists(),
        "and so is the archive it came from"
    );

    let resolved = resolve_tool(
        &serde_json::from_value(demo_glob()).expect("the glob parses"),
        &rig.env,
    )
    .await
    .expect("the resolver runs");
    assert_eq!(
        resolved.map(|found| found.path),
        None,
        "a half-done install must be invisible to the probe, not merely unlikely to be found"
    );
}

/// Hazard H-16 at the other end: a registry entry whose `cmd` climbs out of the tree is refused at
/// pre-flight, before consent and before a byte is fetched.
#[tokio::test]
async fn an_entry_whose_cmd_climbs_out_of_the_tree_is_refused_before_consent() {
    let tmp = tempfile::tempdir().expect("a temporary directory");
    let fixture = Fixture::start().await;
    let document = one_entry_registry(&fixture.url("/archive.zip"), "./sub/../x", None);
    fixture.route("/registry.json", registry_route(&document, 300, "\"v1\""));
    let config = InstallConfig::new(fixture.base(), Some(rooted(&tmp)));
    let installer = Installer::new(config.clone()).expect("a client builds");
    let env = boxed(&config, tmp.path(), "linux-x86_64");

    let error = plan(&installer, &demo_row(), &env, Utc::now())
        .await
        .expect_err("a cmd that leaves the version directory is not installable");

    assert!(
        matches!(&error, PlanError::BadCmd { cmd } if cmd == "./sub/../x"),
        "unexpected refusal: {error:?}"
    );
    assert_eq!(
        fixture.lines(),
        vec!["GET /registry.json".to_owned()],
        "and it costs one document read, not a download"
    );
}

// ---------------------------------------------------------------------------------------------
// T6 fixtures: the two tier-2 seams, a row that resolves what is installed, and the pipeline
// ---------------------------------------------------------------------------------------------

/// The buffer each half of the in-process pipe gets, as `tests/probe.rs` sizes it.
const DUPLEX_BYTES: usize = 64 * 1024;

/// Line 1 of the recorded `claude` transcript: the `initialize` **result** a real adapter sent.
///
/// Copied here rather than shared with `tests/probe.rs`, which is this repository's rule for test
/// helpers: an integration test binary is its own crate, and a `mod common` would make two
/// suites move together for no gain.
fn fixture_initialize_result() -> Value {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude_acp_handshake.jsonl");
    let text = std::fs::read_to_string(path).expect("the recorded handshake fixture");
    let line = text.lines().next().expect("the fixture has a first line");
    let document: Value = serde_json::from_str(line).expect("the first line is JSON");
    document
        .get("result")
        .cloned()
        .expect("line 1 records the initialize result")
}

/// An agent that answers exactly one request — `initialize` — with `result`, then reads until the
/// client goes away.
async fn scripted_initialize(stream: tokio::io::DuplexStream, result: Value) {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = tokio::io::BufReader::new(reader).lines();
    let Ok(Some(line)) = lines.next_line().await else {
        return;
    };
    let request: Value = serde_json::from_str(&line).expect("the client speaks JSON-RPC");
    let response = json!({
        "jsonrpc": "2.0",
        "id": request.get("id").cloned().unwrap_or(Value::Null),
        "result": result,
    });
    let mut text = serde_json::to_string(&response).expect("the response serialises");
    text.push('\n');
    if writer.write_all(text.as_bytes()).await.is_err() {
        return;
    }
    let _ = writer.flush().await;
    while let Ok(Some(_)) = lines.next_line().await {}
}

/// A tier 2 that fails without a process: the seam that lets a case be about what the *installer*
/// does with a `failed` verdict, with no adapter to run and no timing to depend on.
struct FailingTier2(&'static str);

impl Tier2 for FailingTier2 {
    fn handshake<'a>(
        &'a self,
        _launch: &'a ResolvedLaunch,
        _settings: &'a AcpSettings,
        _env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake> {
        Box::pin(async move { Err(DriverError::Transport(self.0.to_owned())) })
    }
}

/// A tier 2 that runs the **real** `acp::handshake` over a duplex whose far end answers `result`:
/// everything but the spawn.
struct DuplexTier2(Value);

impl Tier2 for DuplexTier2 {
    fn handshake<'a>(
        &'a self,
        _launch: &'a ResolvedLaunch,
        settings: &'a AcpSettings,
        _env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake> {
        Box::pin(async move {
            let (client_end, agent_end) = tokio::io::duplex(DUPLEX_BYTES);
            let (reader, writer) = tokio::io::split(client_end);
            tokio::spawn(scripted_initialize(agent_end, self.0.clone()));
            handshake(
                AcpIo {
                    reader: Box::new(reader),
                    writer: Box::new(writer),
                    child: None,
                },
                settings,
                Duration::from_secs(5),
            )
            .await
        })
    }
}

/// The same, recording whether `watch` still existed **at handshake time**.
///
/// This is how "the previous version is deleted *after* the row was produced" (plan D16(a)) is
/// asserted rather than assumed: retention that ran first would be invisible to an assertion made
/// after `install` returns, because both orders end with the directory gone.
struct WatchingTier2 {
    watch: PathBuf,
    seen: Arc<Mutex<Option<bool>>>,
    inner: DuplexTier2,
}

impl Tier2 for WatchingTier2 {
    fn handshake<'a>(
        &'a self,
        launch: &'a ResolvedLaunch,
        settings: &'a AcpSettings,
        env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake> {
        Box::pin(async move {
            *self.seen.lock().expect("the watcher") = Some(self.watch.exists());
            self.inner.handshake(launch, settings, env).await
        })
    }
}

/// A tier 2 that jams the removal of the promoted tree for a moment, then fails the handshake.
///
/// The stand-in for the Windows behaviour plan D21 names: Defender opens a freshly written `.exe`
/// to scan it, and everything that touches the directory holding it comes back
/// `PermissionDenied` until it lets go. There is no Defender here, so the jam is a `0o555` on
/// `<root>/<id>/` — `remove_dir_all` can empty the version directory but cannot unlink it from a
/// parent it may not write — lifted from a task a moment later.
///
/// It jams once. The rollback is followed by a **second** probe (plan D16(b)), and a seam that
/// re-jammed on that one would be describing a lock that never clears rather than one that does.
#[cfg(unix)]
struct JammingTier2 {
    agent_dir: PathBuf,
    clears_after: Duration,
    jammed: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(unix)]
impl Tier2 for JammingTier2 {
    fn handshake<'a>(
        &'a self,
        _launch: &'a ResolvedLaunch,
        _settings: &'a AcpSettings,
        _env: &'a ProbeEnv,
    ) -> DriverFuture<'a, Handshake> {
        Box::pin(async move {
            use std::os::unix::fs::PermissionsExt;

            if !self.jammed.swap(true, std::sync::atomic::Ordering::SeqCst) {
                std::fs::set_permissions(&self.agent_dir, std::fs::Permissions::from_mode(0o555))
                    .expect("the jam is applied");
                let dir = self.agent_dir.clone();
                let after = self.clears_after;
                tokio::spawn(async move {
                    tokio::time::sleep(after).await;
                    let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
                });
            }
            Err(DriverError::Transport(
                "initialize did not answer: jammed".to_owned(),
            ))
        })
    }
}

/// The glob the row declares: the seed shape, with the install root as a token and the version as
/// the one `*`.
fn decoy_glob() -> Value {
    json!({
        "kind": "glob",
        "patterns": [],
        "platform": {
            "linux-x86_64": { "patterns": ["%HTUI_AGENTS_ROOT%/decoy/*/bin/demo-server"] },
        },
    })
}

/// A row whose `command` is the tool its own `discovery` declares, so a complete resolution is a
/// launch tier 2 can be handed.
///
/// [`row`]'s `${tool}` placeholder names no declared tool, which is fine for T4 and T5 — neither
/// spawns anything — but T6 probes for real, and a launch that cannot substitute is `missing`
/// before tier 2 is ever reached.
fn resolving_row(glob: Value) -> Agent {
    let mut agent = row(
        "demo",
        json!({ "demo_server": glob }),
        Some(json!({ "source": "acp_registry", "id": ID, "tool": "demo_server" })),
    );
    agent.launch["command"] = json!("${demo_server}");
    agent
}

/// A `.zip` of many small entries: the one case that needs an unpack long enough to be interrupted
/// while it is running.
fn wide_zip_bytes(count: usize, each: usize) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);
    writer
        .start_file("bin/demo-server", options)
        .expect("the cmd starts");
    std::io::Write::write_all(&mut writer, b"#!/bin/sh\nexit 0\n").expect("the cmd is written");
    let body = vec![b'x'; each];
    for index in 0..count {
        writer
            .start_file(format!("share/part-{index:04}"), options)
            .expect("the entry starts");
        std::io::Write::write_all(&mut writer, &body).expect("the entry is written");
    }
    writer.finish().expect("the archive closes").into_inner()
}

/// A fixture server serving one archive, a row that declares it, and everything [`install`] is
/// told about the box.
struct Pipeline {
    _tmp: tempfile::TempDir,
    installer: Installer,
    plan: htui_agent::InstallPlan,
    layout: Layout,
    env: ProbeEnv,
    agent: Agent,
    box_id: BoxId,
}

impl Pipeline {
    /// The rig, with the pre-flight already run: `/archive.zip` serves `body`, the entry publishes
    /// `sha256`, and the row's `discovery` declares `glob`.
    async fn new(glob: Value, body: Vec<u8>, sha256: Option<String>, route: Route) -> Self {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let fixture = Fixture::start().await;
        let document = one_entry_registry(&fixture.url("/archive.zip"), CMD, sha256.as_deref());
        fixture.route("/registry.json", registry_route(&document, 300, "\"v1\""));
        fixture.route(
            "/archive.zip",
            Route {
                status: 200,
                body,
                ..route
            },
        );
        let mut config = InstallConfig::new(fixture.base(), Some(rooted(&tmp)));
        config.progress_every = Duration::from_millis(1);
        let installer = Installer::new(config.clone()).expect("a client builds");
        let env = boxed(&config, tmp.path(), "linux-x86_64");
        let agent = resolving_row(glob);
        let plan = plan(&installer, &agent, &env, Utc::now())
            .await
            .expect("the pre-flight succeeds");
        let layout = Layout::new(plan.root.clone());
        Self {
            _tmp: tmp,
            installer,
            plan,
            layout,
            env,
            agent,
            box_id: BoxId::new(),
        }
    }

    /// The default rig: the two-entry tree as a `.zip`, digest published, body sent in one go.
    async fn ready() -> Self {
        let body = zip_bytes(&tree());
        let digest = hex_digest(&body);
        Self::new(demo_glob(), body, Some(digest), Route::default()).await
    }

    /// Writes `<root>/<id>/<version>/bin/demo-server`, executable, with `at` as its mtime.
    ///
    /// The mtime is an argument because plan D4's whole point is that it must stop deciding: a
    /// version directory whose file is *newer* than the freshly promoted one still loses.
    fn seed_version(&self, version: &str, at: SystemTime) {
        let cmd = self
            .layout
            .version_dir(&self.plan.registry_id, version)
            .join("bin/demo-server");
        std::fs::create_dir_all(cmd.parent().expect("a parent")).expect("the version directory");
        std::fs::write(&cmd, b"#!/bin/sh\nexit 0\n").expect("the seeded cmd");
        // A file the fixture archive does **not** carry, so "the old tree came back" can be told
        // apart from "the new tree was never removed" — the two are otherwise identical on disk.
        std::fs::write(
            cmd.parent().expect("a parent").join("seeded"),
            b"this tree was here first\n",
        )
        .expect("the marker");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&cmd, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        std::fs::File::options()
            .write(true)
            .open(&cmd)
            .expect("open for mtime")
            .set_modified(at)
            .expect("set mtime");
    }

    /// One `install` call over this rig.
    async fn run(
        &self,
        tier2: &dyn Tier2,
        cancel: &CancellationToken,
        progress: &mut (dyn FnMut(InstallProgress) + Send),
    ) -> Result<InstallOutcome, InstallError> {
        let ctx = ProbeContext {
            env: self.env.clone(),
            now: Utc::now(),
        };
        install(
            &self.installer,
            InstallJob {
                plan: &self.plan,
                agent: &self.agent,
                box_id: self.box_id,
                existing: None,
                ctx: &ctx,
                tier2,
            },
            progress,
            cancel,
        )
        .await
    }

    /// The version directories under the id, sorted.
    async fn versions(&self) -> Vec<String> {
        self.layout.existing_versions(&self.plan.registry_id).await
    }

    /// The names of everything left in `.staging/`, sorted.
    fn staging_entries(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.layout.staging()) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// What the row's own glob resolves on this box right now — no override tier, which is what
    /// the post-promote check asks (hazard H-11).
    async fn glob_resolves(&self) -> Option<PathBuf> {
        let launch: htui_agent::launch::AgentLaunch =
            serde_json::from_value(self.agent.launch.clone()).expect("the row's launch parses");
        let discovery = launch.discovery.expect("the row declares a discovery");
        let probe = discovery
            .tools
            .get(&self.plan.tool)
            .expect("the row declares the install tool");
        resolve_tool(probe, &self.env)
            .await
            .expect("the resolver runs")
            .map(|found| found.path)
    }
}

/// The `resolved.command` of an outcome's row.
fn resolved_command(row: &htui_core::model::AgentBox) -> String {
    ProbeSnapshot::from_row(row)
        .expect("the probe wrote a snapshot")
        .resolved
        .expect("the probe resolved something")
        .command
}

// ---------------------------------------------------------------------------------------------
// T6: the throttle
// ---------------------------------------------------------------------------------------------

/// Plan D18 as a pure decision (hazard H-18): a phase change and a phase's final frame always
/// pass, and nothing else does until `every` has elapsed on the **injected** clock.
///
/// Hand-made instants rather than `#[tokio::test(start_paused = true)]`, because paused time in
/// this file auto-advances past the client timeouts while a socket is not yet readable and every
/// fetch case in the same binary would fail for a reason that has nothing to do with the code.
#[test]
fn the_throttle_passes_a_phase_change_and_a_final_frame_and_otherwise_waits() {
    let base = Instant::now();
    let frame = |phase, done, total| InstallProgress { phase, done, total };
    let mut throttle = Throttle::new(Duration::from_millis(250));

    assert!(
        throttle.admit(frame(InstallPhase::Downloading, 0, Some(100)), base),
        "the first frame of a phase always reaches the cell"
    );
    assert!(
        !throttle.admit(
            frame(InstallPhase::Downloading, 10, Some(100)),
            base + Duration::from_millis(1)
        ),
        "a millisecond later is the flood the 250 ms exists to stop"
    );
    assert!(
        !throttle.admit(
            frame(InstallPhase::Downloading, 20, Some(100)),
            base + Duration::from_millis(249)
        ),
        "and the window is closed right up to its edge"
    );
    assert!(
        throttle.admit(
            frame(InstallPhase::Downloading, 30, Some(100)),
            base + Duration::from_millis(250)
        ),
        "at the edge it opens"
    );
    assert!(
        !throttle.admit(
            frame(InstallPhase::Downloading, 40, Some(100)),
            base + Duration::from_millis(251)
        ),
        "and closes again from the frame that was admitted, not from the phase's start"
    );
    assert!(
        throttle.admit(
            frame(InstallPhase::Downloading, 100, Some(100)),
            base + Duration::from_millis(252)
        ),
        "`done == total` is always admitted: a cell left reading 97% because the last frame fell \
         inside the window is a bug the user reports"
    );
    assert!(
        throttle.admit(
            frame(InstallPhase::Unpacking, 0, None),
            base + Duration::from_millis(253)
        ),
        "a phase change is always admitted: the cell must say what is happening now"
    );
}

// ---------------------------------------------------------------------------------------------
// T6: the pipeline — the row is the probe's, never the installer's
// ---------------------------------------------------------------------------------------------

/// `R-AGT-6`: the outcome's status is the one `probe_agent` wrote, and the installer chose none.
///
/// The row that comes back is the probe's own — its `resolved.command` is the file the archive
/// produced, resolved through the row's glob a second time and not remembered from the unpack.
#[tokio::test]
async fn the_installed_row_is_the_probes_own_and_the_installer_sets_no_status() {
    let rig = Pipeline::ready().await;

    let outcome = rig
        .run(
            &DuplexTier2(fixture_initialize_result()),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .expect("the install runs");

    let InstallOutcome::Installed {
        row,
        status,
        dir,
        record,
        version,
        removed_versions,
        digest_changed,
    } = outcome
    else {
        panic!("a tree the probe handshakes is installed: {outcome:?}");
    };
    assert_eq!(version, VERSION);
    assert_eq!(dir, rig.plan.install_dir);
    assert_eq!(status, ProbeStatus::Ready);
    assert!(removed_versions.is_empty(), "there was nothing to replace");
    assert!(
        !digest_changed,
        "nothing was recorded for this version before"
    );
    assert!(
        record.published,
        "the entry publishes a digest, so the record says the download was verified"
    );

    let snapshot = ProbeSnapshot::from_row(&row).expect("the probe wrote a snapshot");
    assert_eq!(
        snapshot.status, status,
        "the outcome repeats the row's status; it does not decide one"
    );
    assert_eq!(
        resolved_command(&row),
        dir.join("bin/demo-server").to_string_lossy(),
        "the row names the file the archive produced, found by the row's own glob"
    );
    assert!(row.enabled, "`agent_box_row` derives that from the status");
}

/// The PRD's "installer succeeds into a broken tree" metric: a perfect download into a tree the
/// probe cannot handshake ends `Failed`, and it says so with the child's own words.
///
/// This is plan D16(c) as well — there was no previous version, so the tree stays where it is.
/// Rolling back a finished download because a *sibling* CLI is missing would punish the wrong
/// thing; what the user is owed is the probe's text saying which.
#[tokio::test]
async fn a_clean_download_the_probe_cannot_handshake_ends_failed_with_the_tree_left() {
    let rig = Pipeline::ready().await;

    let outcome = rig
        .run(
            &FailingTier2("initialize did not answer: boom"),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .expect("the pipeline itself did not fail: the probe did");

    let InstallOutcome::Failed {
        status,
        stderr_tail,
        restored,
        probe,
        version,
        record,
    } = outcome
    else {
        panic!("a tree that does not handshake is not an install: {outcome:?}");
    };
    assert_eq!(status, ProbeStatus::Failed);
    assert_eq!(version, VERSION);
    assert_eq!(record.sha256, rig.plan.sha256.clone().expect("published"));
    assert_eq!(
        stderr_tail,
        Some(vec!["initialize did not answer: boom".to_owned()]),
        "the failure text is the child's, line by line"
    );
    assert_eq!(
        restored, None,
        "there was no previous version, so there is nothing to have put back"
    );
    assert!(
        matches!(&probe, ProbeOutcome::Row(row) if resolved_command(row)
            == rig.plan.install_dir.join("bin/demo-server").to_string_lossy()),
        "the caller writes the probe's row, which describes the tree that is still there: {probe:?}"
    );
    assert_eq!(
        rig.versions().await,
        vec![VERSION.to_owned()],
        "plan D16(c): the tree stays, because the probe found the binary and not the fault"
    );
    assert!(
        rig.staging_entries().is_empty(),
        "and the archive it came from does not stay: {:?}",
        rig.staging_entries()
    );
}

/// Plan D16(b): a previous version comes back, and the row the caller writes describes **it**.
///
/// The second re-probe is the point. Without it the row would still describe the tree that was
/// just deleted, and the Settings tab would offer to start a binary that is no longer on disk.
#[tokio::test]
async fn a_previous_version_comes_back_and_the_row_describes_what_is_left() {
    let rig = Pipeline::ready().await;
    rig.seed_version("1.1.0", SystemTime::now());

    let outcome = rig
        .run(
            &FailingTier2("initialize did not answer: boom"),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .expect("the pipeline itself did not fail");

    let InstallOutcome::Failed {
        restored, probe, ..
    } = outcome
    else {
        panic!("the new version did not probe usable: {outcome:?}");
    };
    assert_eq!(restored, Some("1.1.0".to_owned()));
    assert_eq!(
        rig.versions().await,
        vec!["1.1.0".to_owned()],
        "1.1.1 is gone and 1.1.0 is back: a failed install never costs the working one"
    );
    let previous = rig
        .layout
        .version_dir(&rig.plan.registry_id, "1.1.0")
        .join("bin/demo-server");
    assert!(
        matches!(&probe, ProbeOutcome::Row(row)
            if resolved_command(row) == previous.to_string_lossy()),
        "the row is the *second* probe's, so it describes 1.1.0: {probe:?}"
    );
    assert!(rig.staging_entries().is_empty(), "and staging is clean");
}

/// Plan D16(a): retention runs **after** the probe has spoken, and the manifest records the
/// install.
#[tokio::test]
async fn a_ready_install_deletes_the_previous_version_after_the_row_was_produced() {
    let rig = Pipeline::ready().await;
    rig.seed_version("1.1.0", SystemTime::now());
    let previous = rig.layout.version_dir(&rig.plan.registry_id, "1.1.0");
    let seen = Arc::new(Mutex::new(None));
    let tier2 = WatchingTier2 {
        watch: previous.clone(),
        seen: Arc::clone(&seen),
        inner: DuplexTier2(fixture_initialize_result()),
    };

    let outcome = rig
        .run(&tier2, &CancellationToken::new(), &mut |_| {})
        .await
        .expect("the install runs");

    let InstallOutcome::Installed {
        removed_versions,
        record,
        ..
    } = outcome
    else {
        panic!("a tree the probe handshakes is installed: {outcome:?}");
    };
    assert_eq!(
        *seen.lock().expect("the watcher"),
        Some(true),
        "the old version is still on disk while the probe runs: deleting first would leave a box \
         with nothing at all if the new tree turned out not to work"
    );
    assert_eq!(removed_versions, vec!["1.1.0".to_owned()]);
    assert!(!previous.exists(), "and afterwards it is gone");
    assert_eq!(rig.versions().await, vec![VERSION.to_owned()]);

    let manifest = Manifest::load(&rig.layout.manifest(&rig.plan.registry_id)).await;
    assert_eq!(
        manifest.installs.get(VERSION),
        Some(&record),
        "the manifest lists the install the outcome reports"
    );
    let consent = manifest
        .consent
        .expect("plan D17: `y` accepted these terms");
    assert_eq!(consent.license.as_deref(), Some("Apache-2.0"));
    assert_eq!(
        consent.license_url.as_deref(),
        Some("https://example.invalid/terms")
    );
    assert_eq!(consent.version, VERSION);
}

/// Plan D16's set-aside, end to end: a re-install of the version that is already there moves the
/// working tree out of the way first, and puts it back when the new one does not probe.
///
/// This is the only path on which `set_aside` and `restore` run inside the pipeline, and the only
/// one where "a failed install never costs the working one" is about the *same* version — the case
/// where a plain rollback would have nothing left to leave behind.
#[tokio::test]
async fn a_same_version_reinstall_that_fails_to_probe_puts_the_working_tree_back() {
    let rig = Pipeline::ready().await;
    rig.seed_version(VERSION, SystemTime::now());
    let marker = rig
        .layout
        .version_dir(&rig.plan.registry_id, VERSION)
        .join("bin/seeded");

    let outcome = rig
        .run(
            &FailingTier2("initialize did not answer: boom"),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .expect("the pipeline itself did not fail");

    let InstallOutcome::Failed { restored, .. } = outcome else {
        panic!("the new tree did not probe usable: {outcome:?}");
    };
    assert_eq!(restored, Some(VERSION.to_owned()));
    assert_eq!(rig.versions().await, vec![VERSION.to_owned()]);
    assert_eq!(
        std::fs::read_to_string(&marker).expect("the working tree is back"),
        "this tree was here first\n",
        "what is on disk is the tree that was set aside, not the one that failed to probe"
    );
    assert!(
        rig.staging_entries().is_empty(),
        "and the `.previous` copy went with the restore: {:?}",
        rig.staging_entries()
    );
}

/// The other half of the set-aside, which every case above tested only through its failure path:
/// a same-version re-install that **succeeds** leaves nothing in `.staging/`.
///
/// `retain_only` walks `<root>/<id>/` and the set-aside copy is not there, so an install that goes
/// well used to leave a full duplicate of the tree — two gigabytes for the largest published
/// adapter — waiting an hour for a sweep. The disk is the smaller half. The larger one is that the
/// copy outlives the version directory it was made from: install this version twice, install a
/// newer one, and D16(a)'s retention deletes `<id>/<version>/` while the `.previous` is still
/// there. The next sweep finds a `.previous` whose target is absent, reads that as "an abort
/// stranded a working version", and **restores** it at any age — the deleted version back on disk
/// with nothing having asked for it.
#[tokio::test]
async fn a_same_version_reinstall_that_succeeds_leaves_no_copy_in_staging() {
    let rig = Pipeline::ready().await;
    rig.seed_version(VERSION, SystemTime::now());

    let outcome = rig
        .run(
            &DuplexTier2(fixture_initialize_result()),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .expect("the install runs");

    assert!(
        matches!(outcome, InstallOutcome::Installed { .. }),
        "the tree probes usable: {outcome:?}"
    );
    assert_eq!(rig.versions().await, vec![VERSION.to_owned()]);
    assert!(
        !rig.layout
            .version_dir(&rig.plan.registry_id, VERSION)
            .join("bin/seeded")
            .exists(),
        "and what is on disk is the tree that was just installed"
    );
    assert!(
        rig.staging_entries().is_empty(),
        "a `.previous` that outlives the install it was made for is a duplicate tree today and a \
         resurrected version at the next sweep: {:?}",
        rig.staging_entries()
    );
}

/// The rollback of plan D16(b) does not give up on the first refusal.
///
/// A removal that fails leaves the rollback half done: the promoted tree is still there, the
/// `.previous` beside it is not put back, and `restored` has already told the caller which version
/// the box was left with. That state is worse than a leak — the next sweep sees a `.previous`
/// whose version directory is *occupied*, reads it as "the promote did happen", and deletes the
/// working copy once it is an hour old. So the removal is retried on the promote's own backoff,
/// which is the same refusal seen from the other side.
///
/// Unix only: the jam is a permission bit, and it stands in for the Windows case (plan D21) that
/// cannot be run here.
#[cfg(unix)]
#[tokio::test]
async fn a_rollback_whose_removal_is_refused_retries_before_it_gives_up() {
    let rig = Pipeline::ready().await;
    rig.seed_version(VERSION, SystemTime::now());
    let agent_dir = rig.layout.agent_dir(&rig.plan.registry_id);
    let marker = rig
        .layout
        .version_dir(&rig.plan.registry_id, VERSION)
        .join("bin/seeded");
    let tier2 = JammingTier2 {
        agent_dir: agent_dir.clone(),
        // Inside the second backoff: the first two attempts are refused, the third is not.
        clears_after: Duration::from_millis(250),
        jammed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };

    let outcome = rig
        .run(&tier2, &CancellationToken::new(), &mut |_| {})
        .await
        .expect("the pipeline itself did not fail");

    let InstallOutcome::Failed { restored, .. } = outcome else {
        panic!("the new tree did not probe usable: {outcome:?}");
    };
    assert_eq!(restored, Some(VERSION.to_owned()));
    assert_eq!(
        std::fs::read_to_string(&marker).expect("the working tree is back"),
        "this tree was here first\n",
        "a single attempt leaves the promoted tree in place, so the version that was set aside \
         never comes back and the row's `restored` is a claim about a directory that is not there"
    );
    assert!(
        rig.staging_entries().is_empty(),
        "and the `.previous` went with the restore rather than waiting for a sweep that would \
         delete it: {:?}",
        rig.staging_entries()
    );
}

/// Plan D4 in situ, which is why T3 exists: the version that wins is the one the *name* ranks
/// highest, not the one `unzip` gave the newest mtime.
///
/// A vendor archive preserves its own build times, so a freshly promoted 1.1.1 routinely carries
/// an mtime older than a 1.1.0 that was written months later. Under milestone 5's rule the glob
/// would answer 1.1.0, the post-promote check would refuse a perfectly good install, and the
/// retention would then delete the version the box was actually resolving.
#[tokio::test]
async fn the_reprobe_resolves_the_new_version_although_the_old_one_is_newer_on_disk() {
    let rig = Pipeline::ready().await;
    rig.seed_version(
        "1.1.0",
        SystemTime::now() + Duration::from_secs(48 * 60 * 60),
    );

    let outcome = rig
        .run(
            &DuplexTier2(fixture_initialize_result()),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .expect("the install runs");

    let InstallOutcome::Installed { row, dir, .. } = outcome else {
        panic!("the post-promote check agreed and the probe handshook: {outcome:?}");
    };
    assert_eq!(
        resolved_command(&row),
        dir.join("bin/demo-server").to_string_lossy(),
        "1.1.1 outranks 1.1.0 by version, whatever the two mtimes say"
    );
}

// ---------------------------------------------------------------------------------------------
// T6: the post-promote check (hazard H-4)
// ---------------------------------------------------------------------------------------------

/// Hazard H-4: the installer wrote a tree the row's own glob does not look at, and says so by
/// naming both paths rather than leaving a gigabyte on disk beside a row that reads `missing`.
///
/// The rollback is D16(b)'s: the promoted tree goes, the previous version is what the box is left
/// with. The check deliberately goes through `resolve_tool` and not `probe_tools` — it asks
/// whether the **glob** sees the file, not what the box would run (hazard H-11).
#[tokio::test]
async fn a_promote_the_rows_glob_cannot_see_is_refused_by_name_and_rolled_back() {
    let body = zip_bytes(&tree());
    let digest = hex_digest(&body);
    let rig = Pipeline::new(decoy_glob(), body, Some(digest), Route::default()).await;
    rig.seed_version("1.1.0", SystemTime::now());

    // Somewhere else entirely: what the row's glob does look at, so the refusal can name it.
    let decoy = rig
        .plan
        .root
        .join("decoy")
        .join("9.9.9")
        .join("bin/demo-server");
    std::fs::create_dir_all(decoy.parent().expect("a parent")).expect("the decoy directory");
    std::fs::write(&decoy, b"not the install").expect("the decoy");

    let error = rig
        .run(
            &FailingTier2("tier 2 must not be reached: the check runs before the probe"),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .expect_err("a promote the row cannot see is not an outcome, it is a refusal");

    let InstallError::NotWhereTheRowLooks { promoted, resolved } = &error else {
        panic!("unexpected refusal: {error:?}");
    };
    assert_eq!(
        promoted.as_path(),
        rig.plan.install_dir.join("bin/demo-server").as_path(),
        "the message names what was written"
    );
    assert_eq!(
        resolved.as_deref(),
        Some(decoy.as_path()),
        "and what the row's glob answered instead"
    );
    assert_eq!(
        rig.versions().await,
        vec!["1.1.0".to_owned()],
        "rolled back as in D16(b): the promoted tree is gone and the previous one is what is left"
    );
    assert!(rig.staging_entries().is_empty(), "and staging is clean");
}

// ---------------------------------------------------------------------------------------------
// T6: cancellation (hazard H-6)
// ---------------------------------------------------------------------------------------------

/// A token tripped while the body is in flight ends `Cancelled` with nothing to sweep.
///
/// The fixture pauses between the two halves of the body, so the token is tripped while
/// `download` is parked on its `select!` — the arm that makes `x` take effect on a stalled body at
/// once rather than at the next byte.
#[tokio::test]
async fn a_token_tripped_mid_download_ends_cancelled_with_staging_swept() {
    let body = zip_bytes(&tree());
    let digest = hex_digest(&body);
    let rig = Pipeline::new(
        demo_glob(),
        body,
        Some(digest),
        Route {
            chunk_delay: Duration::from_millis(600),
            ..Route::default()
        },
    )
    .await;

    let cancel = CancellationToken::new();
    let tripping = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            cancel.cancel();
        })
    };
    let error = rig
        .run(
            &FailingTier2("tier 2 must not be reached: the download was cancelled"),
            &cancel,
            &mut |_| {},
        )
        .await
        .expect_err("a tripped token stops the pipeline");
    tripping.await.expect("the canceller");

    assert!(matches!(error, InstallError::Cancelled), "{error:?}");
    assert!(
        rig.staging_entries().is_empty(),
        "a 400 MB partial waiting an hour for the sweep is exactly what `x` is pressed to avoid: \
         {:?}",
        rig.staging_entries()
    );
    assert!(!rig.plan.install_dir.exists(), "nothing was promoted");
    assert_eq!(
        rig.glob_resolves().await,
        None,
        "and the row's glob resolves nothing: an interrupted install is invisible, not merely \
         unlikely to be found"
    );
}

/// The same, tripped while the **unpack** is running: `spawn_blocking` cannot be aborted, so the
/// token is what stops it, and the half-written tree goes with it (blueprint P-3, hazard H-8).
///
/// The trip is driven by the progress stream rather than by a sleep: the first `Unpacking` frame
/// can only be emitted from the poll loop *while the blocking handle is still unfinished*, so
/// cancelling on it lands inside `unpack`'s per-entry check by construction.
#[tokio::test]
async fn a_token_tripped_mid_unpack_ends_cancelled_with_staging_swept() {
    let body = wide_zip_bytes(400, 32 * 1024);
    let digest = hex_digest(&body);
    let rig = Pipeline::new(demo_glob(), body, Some(digest), Route::default()).await;

    let cancel = CancellationToken::new();
    let mut unpacking = 0_u32;
    let error = rig
        .run(
            &FailingTier2("tier 2 must not be reached: the unpack was cancelled"),
            &cancel,
            &mut |frame| {
                if frame.phase == InstallPhase::Unpacking {
                    unpacking += 1;
                    cancel.cancel();
                }
            },
        )
        .await
        .expect_err("a tripped token stops the pipeline");

    assert!(
        unpacking > 0,
        "the poller has to have seen the unpack running, or this case proves nothing"
    );
    assert!(matches!(error, InstallError::Cancelled), "{error:?}");
    assert!(
        rig.staging_entries().is_empty(),
        "the half-written tree goes with it: {:?}",
        rig.staging_entries()
    );
    assert!(!rig.plan.install_dir.exists(), "nothing was promoted");
    assert_eq!(rig.glob_resolves().await, None);
}

// ---------------------------------------------------------------------------------------------
// T6: the override, and the progress stream
// ---------------------------------------------------------------------------------------------

/// Hazard H-11: `HTUI_TOOL_<NAME>` still wins after an install.
///
/// The re-probe goes through `probe_tools`, whose first tier is the **checked** override, so the
/// row records the file the user pinned. The post-promote check does not, on purpose: it asks
/// whether the glob sees what was written, and an override that answered it would let a typo in a
/// seed pattern ship unnoticed behind a variable the user happens to have set.
#[tokio::test]
async fn the_tool_override_still_wins_after_an_install() {
    let mut rig = Pipeline::ready().await;
    let pinned = rig.plan.root.join("elsewhere").join("demo-server");
    std::fs::create_dir_all(pinned.parent().expect("a parent")).expect("the pinned directory");
    std::fs::write(&pinned, b"#!/bin/sh\nexit 0\n").expect("the pinned tool");
    rig.env.vars.insert(
        htui_agent::env_override_key("demo_server"),
        pinned.to_string_lossy().into_owned(),
    );

    let outcome = rig
        .run(
            &DuplexTier2(fixture_initialize_result()),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .await
        .expect("the install runs: the glob still sees what was written");

    let InstallOutcome::Installed { row, dir, .. } = outcome else {
        panic!("the post-promote check used the glob, not the override: {outcome:?}");
    };
    assert!(
        dir.join("bin/demo-server").exists(),
        "the tree was written and promoted"
    );
    assert_eq!(
        resolved_command(&row),
        pinned.to_string_lossy(),
        "and the row records what the box will actually run"
    );
}

/// The frames the sink sees: every phase in order, `done` never going backwards inside one, and
/// each phase ending on a frame the throttle cannot have swallowed.
#[tokio::test]
async fn the_progress_stream_is_ordered_monotonic_and_ends_each_phase() {
    let rig = Pipeline::ready().await;
    let mut frames = Vec::new();

    let outcome = rig
        .run(
            &DuplexTier2(fixture_initialize_result()),
            &CancellationToken::new(),
            &mut |frame| frames.push(frame),
        )
        .await
        .expect("the install runs");
    assert!(
        matches!(outcome, InstallOutcome::Installed { .. }),
        "the frames under test are a successful install's: {outcome:?}"
    );

    let phases: Vec<InstallPhase> = frames.iter().map(|frame| frame.phase).collect();
    let mut order = Vec::new();
    for phase in phases {
        if order.last() != Some(&phase) {
            order.push(phase);
        }
    }
    assert_eq!(
        order,
        vec![
            InstallPhase::Downloading,
            InstallPhase::Verifying,
            InstallPhase::Unpacking,
            InstallPhase::Probing,
        ],
        "each phase is announced once it starts, and never again after the next one has: {frames:?}"
    );
    for pair in frames.windows(2) {
        if pair[0].phase == pair[1].phase {
            assert!(
                pair[1].done >= pair[0].done,
                "a progress bar that goes backwards is a bug report: {frames:?}"
            );
        }
    }
    let download_end = frames
        .iter()
        .rfind(|frame| frame.phase == InstallPhase::Downloading)
        .expect("the download reported something");
    assert_eq!(
        download_end.total,
        Some(download_end.done),
        "the last frame of a phase says so, so the cell is never left reading 97%: {frames:?}"
    );
}

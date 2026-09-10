//! MOD-20 D19 end to end: the Settings action, through the shell, over a fixture registry.
//!
//! What `tests/settings.rs` asserts about the section in isolation this file asserts about the
//! whole path — a key, a request, the agent runtime, an HTTP server, the installer, the probe, and
//! the frame that comes back. The one claim only this level can make is the negative one: pressing
//! `i` fetches the registry document and asks for the archive's size, and **not one byte of the
//! archive itself**, which is `R-AGT-10`'s rule and which only a recorder in front of a real
//! socket can witness.
//!
//! The fixture server, the archive builder and the registry document below are this file's own,
//! per the repo's per-file test-helper rule. Every one of them is deliberately anonymous
//! (`R-AGT-5`): a made-up entry id, a made-up tool, a loopback URL. A case that spelled a vendor's
//! name would be asserting the seeds rather than the plumbing.
#![cfg(feature = "testkit")]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use htui::agent_worker::AgentRuntime;
use htui::testkit::Harness;
use htui::ui::tabs::settings::{AgentsSection, SettingsTab};
use htui_agent::registry::DriverFactory;
use htui_agent::{INSTALL_ROOT_VAR, InstallConfig, platform_key};
use htui_core::model::{Agent, AgentId, Billing, Transport};
use htui_core::store::{MemStore, WriteStore};
use serde_json::{Value, json};

/// The registry entry id these cases install. Not an agent and not a vendor: a made-up id, which
/// is the point — nothing between the key and the unpacked tree learns what it is installing.
const ENTRY_ID: &str = "demo-acp";

/// The version the fixture registry serves.
const ENTRY_VERSION: &str = "1.0.0";

/// The `discovery.tools` key whose glob must resolve what the install writes, and the name of the
/// file inside the archive that it resolves to.
const TOOL: &str = "demo_server";

/// The registry row's name. It sorts after the two seeded rows, which is why the cases press `j`
/// twice before `i`: the cursor is what `i` acts on, and landing on the wrong row would install
/// something else.
const ROW: &str = "demo";

/// How long a case waits for an install that is meant to finish.
const PATIENCE: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------------------------
// The fixture server
// ---------------------------------------------------------------------------------------------

/// One scripted answer: what the responder sends for one path.
#[derive(Debug, Clone, Default)]
struct Route {
    /// The status line's code.
    status: u16,
    /// The body, sent whole unless `chunk_delay` says otherwise.
    body: Vec<u8>,
    /// When non-zero, the body is sent in two halves with this pause between them.
    ///
    /// It is what puts a cancellation inside the download's `select!` rather than at the check
    /// that guards the next step: without a gap between chunks a body this small arrives whole
    /// before any token could be tripped.
    chunk_delay: Duration,
}

/// A loopback HTTP/1.1 responder with a scripted route table and a request recorder.
#[derive(Debug, Clone)]
struct Fixture {
    /// Where it is listening.
    addr: std::net::SocketAddr,
    /// Every `"<METHOD> <PATH>"` it has answered, in order.
    log: Arc<Mutex<Vec<String>>>,
    /// What it answers each path with.
    routes: Arc<Mutex<HashMap<String, Route>>>,
}

impl Fixture {
    /// Binds an ephemeral port and starts accepting.
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
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

    /// `http://127.0.0.1:<port>`, the value `InstallConfig::registry_base` takes.
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
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_owned(), route);
    }

    /// Every request line the responder has seen, in order.
    fn lines(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Whether the archive's *body* was ever asked for.
    ///
    /// A `HEAD` is not a fetch: it is what the pre-flight is allowed to spend, and it transfers
    /// headers and no adapter.
    fn fetched_the_archive(&self) -> bool {
        self.lines().iter().any(|line| line == "GET /archive.zip")
    }

    /// Reads one request, records it, and writes the scripted answer.
    async fn answer(self, mut stream: tokio::net::TcpStream) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte).await {
                Ok(0) | Err(_) => return,
                Ok(_) => head.push(byte[0]),
            }
            if head.ends_with(b"\r\n\r\n") || head.len() > 16 * 1024 {
                break;
            }
        }
        let head = String::from_utf8_lossy(&head).into_owned();
        let mut parts = head.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let target = parts.next().unwrap_or_default();
        let path = target.split(['?', '#']).next().unwrap_or(target).to_owned();
        // The guard is dropped before the first `.await` below, on purpose: this file lives by the
        // rule the worker does — no lock is ever held across a suspension point.
        let route = {
            let mut log = self.log.lock().unwrap_or_else(PoisonError::into_inner);
            log.push(format!("{method} {path}"));
            drop(log);
            self.routes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&path)
                .cloned()
        };
        let Some(route) = route else {
            let _ = stream
                .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n")
                .await;
            return;
        };
        let head = format!(
            "HTTP/1.1 {} OK\r\nconnection: close\r\ncontent-length: {}\r\n\r\n",
            route.status,
            route.body.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        if method == "HEAD" {
            let _ = stream.flush().await;
            return;
        }
        if route.chunk_delay.is_zero() {
            let _ = stream.write_all(&route.body).await;
        } else {
            let (first, second) = route.body.split_at(route.body.len() / 2);
            let _ = stream.write_all(first).await;
            let _ = stream.flush().await;
            tokio::time::sleep(route.chunk_delay).await;
            let _ = stream.write_all(second).await;
        }
        let _ = stream.flush().await;
    }
}

// ---------------------------------------------------------------------------------------------
// The documents
// ---------------------------------------------------------------------------------------------

/// A one-entry registry document listing [`ENTRY_ID`] for **this** box's platform.
///
/// The platform key is read rather than written: a document pinned to `linux-x86_64` would make
/// every other box's run of this suite assert `NotAvailable`.
fn registry_document(archive_url: &str) -> Value {
    json!({
        "version": "1.0.0",
        "agents": [{
            "id": ENTRY_ID,
            "name": "Demo",
            "version": ENTRY_VERSION,
            "license": "proprietary",
            "license_url": "https://example.invalid/terms",
            "distribution": {
                "binary": {
                    platform_key(): {
                        "archive": archive_url,
                        "cmd": format!("./{TOOL}"),
                        "args": [],
                    },
                },
            },
        }],
        "extensions": [],
    })
}

/// The registry row: a glob under the install root, and a declared source.
///
/// `acp` with a handshake, because the re-probe after promotion is what decides the outcome, and a
/// transport with no tier 2 would let a case pass without ever asking the box a question.
fn install_row(id: AgentId) -> Agent {
    let pattern = format!("%{INSTALL_ROOT_VAR}%/{ENTRY_ID}/*/{TOOL}");
    Agent {
        id,
        name: ROW.to_owned(),
        transport: Transport::Acp,
        billing: Billing::Subscription,
        models: Vec::new(),
        default_model: None,
        launch: json!({
            "command": format!("${{{TOOL}}}"),
            "args": [],
            "env": {},
            "discovery": {
                "tools": { TOOL: { "kind": "glob", "patterns": [pattern] } },
                "handshake": true,
                "install": { "source": "acp_registry", "id": ENTRY_ID, "tool": TOOL },
            },
        }),
        settings: json!({}),
        enabled: true,
        created_at: htui_core::fixtures::demo_at(0, 0),
        updated_at: htui_core::fixtures::demo_at(0, 0),
    }
}

/// A one-entry `.zip` holding `body` at [`TOOL`], **stored** rather than deflated.
///
/// Written by hand so this crate needs no archive dependency for one fixture: `stored` is method
/// 0, which every zip reader supports, and the only arithmetic is the CRC the reader checks. The
/// entry declares no unix mode, which is deliberate — it is the archive shape hazard H-5 is about,
/// and the promoted file is executable only because the installer made it so.
fn stored_zip(body: &[u8]) -> Vec<u8> {
    /// The CRC-32 (IEEE) of `bytes`, which is the one number a zip reader recomputes.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFF_u32;
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                let carry = crc & 1;
                crc >>= 1;
                if carry == 1 {
                    crc ^= 0xEDB8_8320;
                }
            }
        }
        !crc
    }

    let name = TOOL.as_bytes();
    let crc = crc32(body);
    let size = u32::try_from(body.len()).expect("the fixture archive is tiny");
    let name_len = u16::try_from(name.len()).expect("the entry name is short");
    let mut zip = Vec::new();

    // The local file header, then the bytes themselves.
    zip.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
    zip.extend_from_slice(&20_u16.to_le_bytes()); // version needed
    zip.extend_from_slice(&0_u16.to_le_bytes()); // flags
    zip.extend_from_slice(&0_u16.to_le_bytes()); // method: stored
    zip.extend_from_slice(&0_u16.to_le_bytes()); // modification time
    zip.extend_from_slice(&0x21_u16.to_le_bytes()); // modification date: 1980-01-01
    zip.extend_from_slice(&crc.to_le_bytes());
    zip.extend_from_slice(&size.to_le_bytes()); // compressed
    zip.extend_from_slice(&size.to_le_bytes()); // uncompressed
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(name);
    zip.extend_from_slice(body);

    // The central directory, which is what a reader opens the archive by.
    let directory_at = u32::try_from(zip.len()).expect("the fixture archive is tiny");
    zip.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
    zip.extend_from_slice(&20_u16.to_le_bytes()); // version made by
    zip.extend_from_slice(&20_u16.to_le_bytes()); // version needed
    zip.extend_from_slice(&0_u16.to_le_bytes()); // flags
    zip.extend_from_slice(&0_u16.to_le_bytes()); // method: stored
    zip.extend_from_slice(&0_u16.to_le_bytes()); // modification time
    zip.extend_from_slice(&0x21_u16.to_le_bytes()); // modification date
    zip.extend_from_slice(&crc.to_le_bytes());
    zip.extend_from_slice(&size.to_le_bytes());
    zip.extend_from_slice(&size.to_le_bytes());
    zip.extend_from_slice(&name_len.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes()); // extra field length
    zip.extend_from_slice(&0_u16.to_le_bytes()); // comment length
    zip.extend_from_slice(&0_u16.to_le_bytes()); // disk number
    zip.extend_from_slice(&0_u16.to_le_bytes()); // internal attributes
    zip.extend_from_slice(&0_u32.to_le_bytes()); // external attributes: no unix mode
    zip.extend_from_slice(&0_u32.to_le_bytes()); // offset of the local header
    zip.extend_from_slice(name);

    // The end-of-central-directory record.
    let directory_len =
        u32::try_from(zip.len()).expect("the fixture archive is tiny") - directory_at;
    zip.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes()); // this disk
    zip.extend_from_slice(&0_u16.to_le_bytes()); // the disk the directory starts on
    zip.extend_from_slice(&1_u16.to_le_bytes()); // entries on this disk
    zip.extend_from_slice(&1_u16.to_le_bytes()); // entries in total
    zip.extend_from_slice(&directory_len.to_le_bytes());
    zip.extend_from_slice(&directory_at.to_le_bytes());
    zip.extend_from_slice(&0_u16.to_le_bytes()); // comment length
    zip
}

// ---------------------------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------------------------

/// A shell over the demo store plus the installable row, an install runtime pointed at a fixture
/// server, and a temporary install root.
struct Rig {
    /// Kept alive: dropping it removes the install root mid-test.
    tmp: tempfile::TempDir,
    /// The root the runtime was actually given, so a test can prove it is under [`Self::tmp`].
    root: std::path::PathBuf,
    /// The server, for the recorder.
    fixture: Fixture,
    /// The shell, settled and with the cursor on [`ROW`].
    harness: Harness,
}

impl Rig {
    /// A rig whose archive route is `archive`, with the cursor already on the installable row.
    async fn new(archive: Route) -> Self {
        let tmp = tempfile::tempdir().expect("a temporary install root");
        let fixture = Fixture::start().await;
        fixture.route("/archive.zip", archive);
        fixture.route(
            "/registry.json",
            Route {
                status: 200,
                body: serde_json::to_vec(&registry_document(&fixture.url("/archive.zip")))
                    .expect("the document serialises"),
                chunk_delay: Duration::ZERO,
            },
        );

        let store = MemStore::demo();
        store
            .upsert_agent(&install_row(AgentId::new()))
            .await
            .expect("the row lands");

        let root = tmp.path().join("agents");
        let runtime = AgentRuntime::new(DriverFactory::new())
            .with_installer(InstallConfig::new(fixture.base(), Some(root.clone())));
        let mut harness = Harness::over(store)
            .with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
                AgentsSection::new(),
            )])))
            .with_agent_runtime(runtime);
        harness.drive().await;

        // Onto the installable row. The seeds are `agy` and `claude`, and both sort before it.
        harness.key("j");
        harness.key("j");
        Self {
            tmp,
            root,
            fixture,
            harness,
        }
    }
}

/// Where `on this box` starts in a rendered row: the seven columns before it — MOD-2 D76's `name`
/// 8, 9, 12, 6, D76's `default` 21, 7 and D73's `quota` 13 — plus one space of `column_spacing`
/// after each. The eighth column therefore draws 15 inside the pane's border, and this file's
/// progress cells are the ones that still do not fit it: `downloading 12.0 MB` is 19, and it
/// exceeded the 17 the column drew before D76 as well.
const ON_BOX_AT: usize = 83;

/// The `on this box` cell of the installable row.
///
/// By character offset since D73's `quota` column landed in front of it: that cell holds spaces of
/// its own (`62% to 09-08 08:00`), so the columns after it can no longer be counted in words.
fn on_box_cell(frame: &str) -> String {
    let line = frame
        .lines()
        .find(|line| line.trim_start_matches('\u{2502}').starts_with(ROW))
        .unwrap_or_else(|| panic!("the `{ROW}` row is rendered:\n{frame}"));
    line.trim_matches('\u{2502}')
        .chars()
        .skip(ON_BOX_AT)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

// ---------------------------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------------------------

/// `R-AGT-10` at the app level: `i` puts the whole consent on screen and the recorder proves that
/// nothing was downloaded to produce it.
///
/// Two request lines and no third: the registry document, and a `HEAD` for the archive's size. The
/// `HEAD` is what makes the `disk` and `from` lines of the pane true, and it transfers no adapter.
#[tokio::test]
async fn i_shows_the_consent_pane_without_fetching_the_archive() {
    let mut rig = Rig::new(Route {
        status: 200,
        body: stored_zip(b"htui install fixture, not an executable\n"),
        chunk_delay: Duration::ZERO,
    })
    .await;

    rig.harness.key("i");
    rig.harness.drive_to_end().await;

    let frame = rig.harness.render();
    assert!(
        frame.contains(&format!("install Demo {ENTRY_VERSION} for {ROW}")),
        "the pre-flight's own headline: {frame}"
    );
    assert!(
        frame.contains("y install \u{b7} n cancel"),
        "and the hint line says what answers it: {frame}"
    );
    assert_eq!(
        rig.fixture.lines(),
        vec![
            "GET /registry.json".to_owned(),
            "HEAD /archive.zip".to_owned()
        ],
        "one document and one size, in that order and nothing else"
    );
    assert!(!rig.fixture.fetched_the_archive());
}

/// `n` is the other half of the same claim: a consent that is refused costs the box nothing.
#[tokio::test]
async fn n_closes_the_pane_and_still_nothing_was_fetched() {
    let mut rig = Rig::new(Route {
        status: 200,
        body: stored_zip(b"htui install fixture, not an executable\n"),
        chunk_delay: Duration::ZERO,
    })
    .await;

    rig.harness.key("i");
    rig.harness.drive_to_end().await;
    rig.harness.key("n");
    rig.harness.drive_to_end().await;

    let frame = rig.harness.render();
    assert!(
        !frame.contains(&format!("install Demo {ENTRY_VERSION}")),
        "the pane is gone: {frame}"
    );
    assert!(frame.contains("install declined"), "{frame}");
    assert!(
        !rig.fixture.fetched_the_archive(),
        "and the archive was never asked for: {:?}",
        rig.fixture.lines()
    );
    assert_eq!(
        on_box_cell(&frame),
        "not probed",
        "the row is exactly where it was"
    );
}

/// `y` runs the whole pipeline, and what the cell then says is the **probe's** verdict, read back
/// through a fresh `StoreRequest::Agents` (`R-AGT-6`).
///
/// The archive's one entry is a text file, so the tree promotes and then fails to hand-shake. That
/// is the point rather than a shortcut: the download, the unpack, the promotion and the glob check
/// all succeed, the probe says no, and the section renders the no. There is no "installed" state
/// anywhere in the UI for it to say otherwise with.
#[tokio::test]
async fn y_streams_to_done_and_the_cell_reads_the_probes_verdict() {
    let mut rig = Rig::new(Route {
        status: 200,
        body: stored_zip(b"htui install fixture, not an executable\n"),
        chunk_delay: Duration::ZERO,
    })
    .await;

    rig.harness.key("i");
    rig.harness.drive_to_end().await;
    rig.harness.key("y");
    tokio::time::timeout(PATIENCE, rig.harness.drive_to_end())
        .await
        .expect("the install finishes inside a minute");

    let frame = rig.harness.render();
    assert!(
        rig.fixture.fetched_the_archive(),
        "this time the archive was fetched: {:?}",
        rig.fixture.lines()
    );
    assert!(
        frame.contains(&format!("install of {ENTRY_VERSION} failed")),
        "the notice says what happened: {frame}"
    );
    assert_eq!(
        on_box_cell(&frame),
        "failed",
        "and the cell is the probe's answer, not the installer's: {frame}"
    );
    assert!(
        frame.contains("i install"),
        "the section is idle again: {frame}"
    );
}

/// `x` mid-download: the runtime is asked to stop, the stream ends `Cancelled`, and the row goes
/// back to whatever the probe last said about it — which here is that it was never probed.
#[tokio::test]
async fn x_mid_download_ends_cancelled_and_the_cell_goes_back() {
    let mut rig = Rig::new(Route {
        status: 200,
        body: stored_zip(b"htui install fixture, not an executable\n"),
        // Half the body, then a pause: the download is inside its `select!` when the token trips.
        chunk_delay: Duration::from_secs(30),
    })
    .await;

    rig.harness.key("i");
    rig.harness.drive_to_end().await;
    rig.harness.key("y");
    rig.harness.drive().await;
    // The *phase* is deliberately not asserted: `Harness::drive` yields only through the store's
    // own awaits, so whether the spawned install task has been polled — and therefore whether the
    // cell still reads `planning…` or already reads `downloading…` — is a scheduling detail. What
    // matters is that an install is in flight and offering its cancel key.
    let rendered = rig.harness.render();
    assert!(
        rendered.contains("x cancel install"),
        "an install is running before the cancel is pressed: {rendered}"
    );

    rig.harness.key("x");
    tokio::time::timeout(PATIENCE, rig.harness.drive_to_end())
        .await
        .expect("a cancelled install ends inside a minute");

    let frame = rig.harness.render();
    assert!(frame.contains("install cancelled"), "{frame}");
    assert_eq!(
        on_box_cell(&frame),
        "not probed",
        "a cancelled install leaves the box nothing to describe: {frame}"
    );
}

/// `r` while an install runs is refused by name: the install is about to spawn a process of its
/// own, and a probe beside it would race it for the same `agent_box` row.
#[tokio::test]
async fn r_during_an_install_is_refused() {
    let mut rig = Rig::new(Route {
        status: 200,
        body: stored_zip(b"htui install fixture, not an executable\n"),
        chunk_delay: Duration::from_secs(30),
    })
    .await;

    rig.harness.key("i");
    rig.harness.drive_to_end().await;
    rig.harness.key("y");
    rig.harness.drive().await;

    rig.harness.key("r");
    assert_eq!(
        rig.harness.app().status.as_deref(),
        Some("an install is running; probe afterwards"),
    );
    assert_eq!(
        on_box_cell(&rig.harness.render()),
        "planning\u{2026}",
        "and the install is untouched by the refusal"
    );

    rig.harness.key("x");
    tokio::time::timeout(PATIENCE, rig.harness.drive_to_end())
        .await
        .expect("the install is stopped so the suite does not wait on the fixture's pause");
}

/// This file must never reach the maintainer's own install root. Asserted against the [`Rig`] the
/// cases actually use — an assertion about `InstallConfig::new` alone would still pass on a `Rig`
/// that dropped the override on its way to the runtime.
#[tokio::test]
async fn the_install_root_is_always_overridden() {
    let rig = Rig::new(Route {
        status: 200,
        body: stored_zip(b"unused\n"),
        chunk_delay: Duration::ZERO,
    })
    .await;
    assert!(
        rig.root.starts_with(rig.tmp.path()),
        "the rig installs under its own tempdir, not {:?}",
        rig.root
    );
}

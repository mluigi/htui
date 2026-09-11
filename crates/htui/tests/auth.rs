//! MOD-21 D20 end to end: `Settings > a`, through the shell, over a fixture agent.
//!
//! What `tests/settings.rs` asserts about the section in isolation this file asserts about the
//! whole path — a key, a request, the agent runtime, a spawned adapter that speaks ACP, the human's
//! choice, the flow's own re-probe, and the frame that comes back. The one claim only this level
//! can make is the one the PRD is written around: after `a` and `Enter` the `on this box` column
//! says what the **probe** found, and nothing between the key and the cell ever decided it.
//!
//! The fixture agent, the registry row and the opener below are this file's own, per the repo's
//! per-file test-helper rule. Every one of them is deliberately anonymous (`R-AGT-5`): a made-up
//! method id, a made-up link, a "credential" that is a sentinel string in a temporary file. A case
//! that spelled a vendor's name would be asserting the seeds rather than the plumbing.
//!
//! Unix-only, because the fixture is a shell script and the pid it writes is what proves the child
//! ran at all.
#![cfg(all(feature = "testkit", unix))]

use std::path::{Path, PathBuf};
use std::time::Duration;

use htui::agent_worker::AgentRuntime;
use htui::testkit::Harness;
use htui::ui::tabs::settings::{AgentsSection, SettingsTab};
use htui_agent::acp::Handshake;
use htui_agent::auth::OpenerCommand;
use htui_agent::probe::{CredentialTier, ProbeSnapshot, ProbeSource, ProbeStatus};
use htui_agent::registry::DriverFactory;
use htui_core::fixtures::ids;
use htui_core::model::{Agent, AgentBox, AgentId, Billing, Transport};
use htui_core::store::{MemStore, WriteStore};
use serde_json::{Value, json};

/// The row's name. It sorts after both seeded agents, which is why every case presses `j` twice
/// before `a`: the cursor is what `a` acts on.
const ROW: &str = "demo-login";

/// Where `on this box` starts in a rendered row: the seven columns before it — MOD-2 D89's `name`
/// 10, then 9, 12, 6, D76's `default` 21, 7 and D73's `quota` 13 — plus one space of
/// `column_spacing` after each.
///
/// **D89 moved this by 2.** `name` became the table's flexible column and `on this box` became the
/// donor that paid for it, fixed at [`ON_BOX_WIDTH`]. Inside the Settings pane's border the section
/// draws 98, so `name` draws 10 here.
const ON_BOX_AT: usize = 85;

/// How wide `on this box` draws since D89: 13 at every width, because it is no longer the column
/// that absorbs the slack. `unauthenticated` is 15 and therefore clips — the stated price of D89,
/// which [`the_name_column_holds_the_whole_row_name_and_on_this_box_pays`] in `tests/settings.rs`
/// records as accepted rather than as damage.
///
/// [`the_name_column_holds_the_whole_row_name_and_on_this_box_pays`]: crate
const ON_BOX_WIDTH: usize = 13;

/// How wide the `name` column draws inside the pane's border (D89): 10, which is what
/// `Constraint::Min(10)` guarantees at the narrowest width the app draws in. [`ROW`] is 10
/// characters and now renders **whole**, where D76's 8 clipped it to `demo-log`.
const NAME_WIDTH: usize = 10;

/// The one method the fixture advertises. A made-up id: the chooser is fed by the agent's own live
/// `initialize`, so a case only ever needs *an* id.
const METHOD: &str = "m-one";

/// What a "credential" is here: a sentinel string the fixture writes into the file its row's
/// `discovery.credential.files` names. The probe `stat`s that file and never reads it.
const CREDENTIAL: &str = "SECRET-SENTINEL";

/// The link the fixture prints to its own stderr, which is what `o` forwards to the opener.
const LINK: &str = "https://h.invalid/login?state=abc";

/// How long a case waits on a flow before calling it stuck rather than slow.
const PATIENCE: Duration = Duration::from_secs(60);

/// How long a case sleeps between drives, so the flow's own task gets to run.
const TICK: Duration = Duration::from_millis(10);

// ---------------------------------------------------------------------------------------------
// The fixture agent
// ---------------------------------------------------------------------------------------------

/// The scripted adapter: a real process, a real environment, and just enough JSON-RPC to answer
/// `initialize`, `authenticate` and `logout`.
///
/// Behaviour by environment, so one script serves every case. `FIXTURE_DIR` is where the pid and
/// the credential go; `FIXTURE_INIT` is the `initialize` result on one line; `FIXTURE_KEY` unset
/// makes `authenticate` refuse the way an adapter refuses a login it has no variable for;
/// `FIXTURE_HOLD` makes it never answer, which is how a case gets to press `o` and `x` while a
/// login is genuinely in flight; `FIXTURE_URL` is a link printed to stderr before the answer;
/// `FIXTURE_CRED` is written into the credential file on success and removed on `logout`.
///
/// The id is echoed back **as it arrived**, quotes and all: this SDK sends a UUID *string* as its
/// JSON-RPC id, and a fixture that assumed a number would answer with a line the client's decoder
/// skips — a handshake timeout wearing a costume.
const AGENT_SH: &str = r#"
echo $$ > "$FIXTURE_DIR/pid"
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\("[^"]*"\|[0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$FIXTURE_INIT" ;;
    *'"method":"authenticate"'*)
      if [ -n "$FIXTURE_URL" ]; then echo "open the following link to log in: $FIXTURE_URL" >&2; fi
      if [ -n "$FIXTURE_HOLD" ]; then sleep 3600; fi
      if [ -z "$FIXTURE_KEY" ]; then
        printf '{"jsonrpc":"2.0","id":%s,"error":{"code":-32602,"message":"the FIXTURE_KEY variable must be set where this server is launched from"}}\n' "$id"
      else
        if [ -n "$FIXTURE_CRED" ]; then printf '%s\n' "$FIXTURE_CRED" > "$FIXTURE_DIR/credential"; fi
        printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
      fi ;;
    *'"method":"logout"'*)
      rm -f "$FIXTURE_DIR/credential"
      printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id" ;;
  esac
done
"#;

/// Writes `contents` at `path` and makes it executable.
///
/// The script is run as `/bin/sh <path>` rather than executed, for `tests/probe.rs`'s `ETXTBSY`
/// reason: a `fork` in another test's spawn inherits every fd open at that instant, and a child
/// holding a write fd makes `execve` refuse until it execs. `sh` only *reads* the file.
fn executable(path: &Path, contents: &str) {
    std::fs::write(path, contents).expect("the fixture script is written");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("the fixture script is executable");
}

/// The `initialize` result the fixture answers with, on one line so the script's `case` globs
/// cannot trip on it.
fn init_result() -> String {
    serde_json::to_string(&json!({
        "protocolVersion": 1,
        "agentInfo": { "name": ROW, "version": "0.0.0" },
        "agentCapabilities": { "loadSession": false, "auth": { "logout": {} } },
        "authMethods": [
            { "id": METHOD, "name": "One", "description": "the fixture's only method" }
        ],
    }))
    .expect("one line of JSON")
}

/// The registry row whose adapter **is** the fixture script.
///
/// `command: "/bin/sh"` with the script as its argument, and a `discovery` that names no tool: tier
/// 1 has nothing to resolve, so the row reaches tier 2 and the re-probe the flow ends with is a
/// real handshake. `credential.files` names the file the fixture writes, which is what turns a
/// completed call into `ready`.
fn login_row(id: AgentId, dir: &Path, extra: &[(&str, &str)]) -> Agent {
    let script = dir.join("agent.sh");
    executable(&script, AGENT_SH);
    let mut env = serde_json::Map::new();
    env.insert(
        "FIXTURE_DIR".to_owned(),
        json!(dir.to_string_lossy().into_owned()),
    );
    env.insert("FIXTURE_INIT".to_owned(), json!(init_result()));
    for (name, value) in extra {
        env.insert((*name).to_owned(), json!(*value));
    }
    Agent {
        id,
        name: ROW.to_owned(),
        transport: Transport::Acp,
        billing: Billing::Subscription,
        models: Vec::new(),
        default_model: None,
        launch: json!({
            "command": "/bin/sh",
            "args": [script.to_string_lossy().into_owned()],
            "env": Value::Object(env),
            "discovery": {
                "tools": {},
                "handshake": true,
                "credential": {
                    "env": [],
                    "files": [dir.join("credential").to_string_lossy().into_owned()],
                },
            },
        }),
        settings: json!({}),
        enabled: true,
        created_at: htui_core::fixtures::demo_at(0, 0),
        updated_at: htui_core::fixtures::demo_at(0, 0),
    }
}

/// This box's `agent_box` for `agent_id`: probed, `unauthenticated`, advertising one method.
///
/// The row a login is *for*, and the row the flow's own re-probe overwrites. `resolved: None`, so
/// the driver resolves the row's own document rather than a recording.
fn probed_login_box(agent_id: AgentId) -> AgentBox {
    let now = htui_core::fixtures::demo_at(0, 0);
    let snapshot = ProbeSnapshot {
        transport: Transport::Acp,
        resolved: None,
        tools: std::collections::BTreeMap::new(),
        handshake: Some(Handshake {
            at: now,
            protocol_version: 1,
            agent_name: Some(ROW.to_owned()),
            agent_version: Some("0.0.0".to_owned()),
            capabilities: json!({}),
            auth_methods: vec![METHOD.to_owned()],
        }),
        credential: Some(CredentialTier::Absent),
        status: ProbeStatus::Unauthenticated,
        stderr_tail: None,
        source: ProbeSource::Probe,
    };
    AgentBox {
        agent_id,
        box_id: ids::BOX,
        enabled: true,
        version: Some("0.0.0".to_owned()),
        path: None,
        probed_at: Some(now),
        quota: None,
        quota_at: None,
        updated_at: now,
        probe: Some(snapshot.to_value()),
    }
}

/// The opener a case injects (blueprint P-5): a script that records the URL it was handed.
///
/// Injected rather than defaulted because the default is the platform's own browser, and a suite
/// that opened the maintainer's browser would be a suite nobody runs twice.
fn recorder(dir: &Path) -> PathBuf {
    let path = dir.join("opener.sh");
    executable(
        &path,
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> \"{}/opened\"\n",
            dir.display()
        ),
    );
    path
}

// ---------------------------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------------------------

/// A shell over the demo store plus the login row, a runtime over the real ACP transport, and a
/// temporary directory the fixture writes everything into.
struct Rig {
    /// Kept alive: dropping it removes the script mid-test.
    tmp: tempfile::TempDir,
    /// For reading the row back the way a fresh `StoreRequest::Agents` would.
    store: MemStore,
    /// The row a login is offered on.
    agent_id: AgentId,
    /// The shell, settled and with the cursor on [`ROW`].
    harness: Harness,
}

impl Rig {
    /// A rig whose fixture is configured by `extra`, with the cursor already on the login row.
    async fn new(extra: &[(&str, &str)]) -> Self {
        let tmp = tempfile::tempdir().expect("a throwaway directory");
        let store = MemStore::demo();
        let agent_id = AgentId::new();
        store
            .upsert_agent(&login_row(agent_id, tmp.path(), extra))
            .await
            .expect("the login row lands");
        store
            .upsert_agent_box(&probed_login_box(agent_id))
            .await
            .expect("the box row lands");

        // The real ACP transport, because the subject is a login that spawns a process: a scripted
        // adapter would answer `Unsupported` and prove nothing.
        let runtime = AgentRuntime::new(DriverFactory::with_acp())
            .with_grace(Duration::ZERO)
            .with_opener(OpenerCommand::Custom(recorder(tmp.path())));
        let mut harness = Harness::over(store.clone())
            .with_tab(Box::new(SettingsTab::with_sections(vec![Box::new(
                AgentsSection::new(),
            )])))
            .with_agent_runtime(runtime);
        harness.drive().await;

        // Onto the login row, counted rather than hard-coded. It used to be two `j`s because the
        // seeds were `agy` and `claude`; MOD-2 D79 added a third that also sorts before
        // `demo-login`, and the cursor landed on a `cli` row whose `authenticate` correctly refuses
        // (MOD-21 D10) — five cases failing on a fixture's arithmetic rather than on their subject.
        // Deriving the count from the store means the next registry addition moves the cursor with
        // it, which is what MOD-23 and MOD-12 are about to do.
        let before = store
            .agents()
            .await
            .expect("the memory store never fails")
            .iter()
            .filter(|summary| summary.agent.name.as_str() < ROW)
            .count();
        for _ in 0..before {
            harness.key("j");
        }
        Self {
            tmp,
            store,
            agent_id,
            harness,
        }
    }

    /// Drives until the rendered frame satisfies `ready`, or fails saying what it was waiting for.
    ///
    /// A login is the one runtime request that can be pending on a **human**: `drive_to_end` awaits
    /// the flow for `CHAT_END` before it does anything else (blueprint P-2), so a case that used it
    /// while a flow was live would sit out the whole limit and then cancel the very thing it was
    /// watching. Alternating a drive with a short sleep is what lets the flow's own task run.
    async fn until(&mut self, what: &str, ready: impl Fn(&str) -> bool) -> String {
        let deadline = std::time::Instant::now() + PATIENCE;
        loop {
            self.harness.drive().await;
            let frame = self.harness.render();
            if ready(&frame) {
                return frame;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the login never {what}:\n{frame}"
            );
            tokio::time::sleep(TICK).await;
        }
    }

    /// `a`, then the chooser the agent's own `initialize` fed.
    async fn start_login(&mut self) {
        self.harness.key("a");
        // `choose a meth`, not `choose a method`: D89 fixed the `on this box` column at
        // [`ON_BOX_WIDTH`] and this prompt is 15 characters, so the frame carries the clipped form.
        // Matching the whole string here would wait out [`PATIENCE`] on a screen that is doing
        // exactly what it was asked to — which is the failure this comment exists to prevent
        // somebody re-introducing.
        self.until("offered a method", |frame| frame.contains("choose a meth"))
            .await;
    }

    /// The login row's line, unbordered, found by the name **as the `name` column drew it**.
    ///
    /// Taken at [`NAME_WIDTH`]: [`ROW`] is exactly that long, so since D89 it draws whole and this
    /// is a prefix of the full name rather than a clip of it.
    fn row_line(&mut self) -> String {
        let frame = self.harness.render();
        let drawn = ROW.chars().take(NAME_WIDTH).collect::<String>();
        frame
            .lines()
            .find(|line| line.trim_start_matches('\u{2502}').starts_with(&drawn))
            .unwrap_or_else(|| panic!("the `{ROW}` row is rendered:\n{frame}"))
            .trim_matches('\u{2502}')
            .to_owned()
    }

    /// The `on this box` cell of the login row, which is the last column of its line.
    ///
    /// By character offset since MOD-2 D73's `quota` column landed in front of it: that cell holds
    /// spaces of its own (`62% to 09-08 08:00`), so the columns after it can no longer be counted
    /// in words.
    fn on_box_cell(&mut self) -> String {
        self.row_line()
            .chars()
            .skip(ON_BOX_AT)
            // Bounded by the column's own width rather than by the end of the line: since D89
            // `on this box` is a `Length(13)` and no longer the column that absorbs the slack, so
            // reading to the end of a wider render would pick up padding that belongs to nobody.
            .take(ON_BOX_WIDTH)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }

    /// The first six columns of the login row, which is the registry's own half of the table.
    fn agent_columns(&mut self) -> String {
        self.row_line()
            .split_whitespace()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// `probe.status` of the row this box now holds, straight from the store.
    async fn stored_status(&self) -> Option<String> {
        self.store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == self.agent_id)
            .and_then(|summary| summary.on_box)
            .and_then(|row| row.probe)
            .and_then(|probe| {
                probe
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
    }

    /// The URLs the injected opener recorded, in order.
    fn opened(&self) -> Vec<String> {
        std::fs::read_to_string(self.tmp.path().join("opened"))
            .unwrap_or_default()
            .lines()
            .map(ToOwned::to_owned)
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------------------------

/// The PRD's own metric, end to end: `a`, a method, and the `on this box` column changes — because
/// the flow re-probed the row and the probe found the credential the agent left behind (D6).
#[tokio::test]
async fn a_then_enter_logs_in_and_the_cell_reads_the_probes_verdict() {
    let mut rig = Rig::new(&[("FIXTURE_KEY", "set"), ("FIXTURE_CRED", CREDENTIAL)]).await;
    assert_eq!(
        rig.on_box_cell(),
        "unauthenticat",
        "the box starts where the probe left it"
    );

    rig.start_login().await;
    rig.harness.key("Enter");
    let frame = rig
        .until("finished", |frame| frame.contains("logged in:"))
        .await;

    assert!(
        frame.contains("logged in: ready"),
        "the notice is the probe's verdict, not the call's: {frame}"
    );
    assert_eq!(
        rig.stored_status().await.as_deref(),
        Some("ready"),
        "and it is the row the flow wrote"
    );
    assert_ne!(
        rig.on_box_cell(),
        "unauthenticat",
        "the cell was re-read from that row: {}",
        rig.harness.render()
    );
    assert!(
        rig.harness.render().contains("a authenticate"),
        "and the section is idle again"
    );
}

/// The other half of the same claim: the agent said yes, left nothing behind, and the column still
/// says `unauthenticated`. There is no state anywhere in the UI that could say otherwise.
#[tokio::test]
async fn a_then_enter_into_no_credential_still_reads_unauthenticated() {
    let mut rig = Rig::new(&[("FIXTURE_KEY", "set")]).await;

    rig.start_login().await;
    rig.harness.key("Enter");
    let frame = rig
        .until("finished", |frame| frame.contains("logged in:"))
        .await;

    assert!(
        frame.contains("logged in: unauthenticated"),
        "the call returned and the box says otherwise: {frame}"
    );
    assert_eq!(
        rig.stored_status().await.as_deref(),
        Some("unauthenticated")
    );
    assert_eq!(rig.on_box_cell(), "unauthenticat");
}

/// `x` mid-login: the runtime is asked to stop, the stream ends `Cancelled`, and the row goes back
/// to whatever the probe last said about it — nothing was written.
#[tokio::test]
async fn x_mid_login_ends_cancelled_and_the_cell_goes_back() {
    let mut rig = Rig::new(&[("FIXTURE_KEY", "set"), ("FIXTURE_HOLD", "1")]).await;

    rig.start_login().await;
    rig.harness.key("Enter");
    let frame = rig
        .until("reached the call", |frame| {
            frame.contains("logging in\u{2026}")
        })
        .await;
    assert!(
        frame.contains("o open link \u{b7} x cancel"),
        "a login is running and offering its cancel key: {frame}"
    );

    rig.harness.key("x");
    tokio::time::timeout(PATIENCE, rig.harness.drive_to_end())
        .await
        .expect("a cancelled login ends inside the patience window");

    let frame = rig.harness.render();
    assert!(frame.contains("login cancelled"), "{frame}");
    assert_eq!(
        rig.on_box_cell(),
        "unauthenticat",
        "a cancelled login leaves the box exactly as it found it: {frame}"
    );
    assert_eq!(
        rig.stored_status().await.as_deref(),
        Some("unauthenticated")
    );
}

/// `o` forwards the link the adapter printed to `htui`'s own opener, and to nothing else: the
/// recorder is what proves the URL travelled and that no browser was launched to find out.
#[tokio::test]
async fn o_opens_the_link_through_the_injected_opener() {
    let mut rig = Rig::new(&[
        ("FIXTURE_KEY", "set"),
        ("FIXTURE_HOLD", "1"),
        ("FIXTURE_URL", LINK),
    ])
    .await;

    rig.start_login().await;
    rig.harness.key("Enter");
    rig.until("printed its link", |frame| {
        frame.contains(&format!("link: {LINK}"))
    })
    .await;

    rig.harness.key("o");
    let frame = rig
        .until("opened the link", |frame| frame.contains("link opened"))
        .await;
    assert_eq!(
        rig.opened(),
        vec![LINK.to_owned()],
        "the opener was handed the adapter's own link and nothing else: {frame}"
    );
    assert!(
        frame.contains("logging in\u{2026}"),
        "and the flow is untouched by it: {frame}"
    );

    rig.harness.key("x");
    tokio::time::timeout(PATIENCE, rig.harness.drive_to_end())
        .await
        .expect("the login is stopped so the suite does not wait on the fixture's sleep");
}

/// `R-AGT-5`'s other half at the app level: a login writes `agent_box` and **never** `agent`. The
/// registry's own columns are the same before and after, and so are the row's bytes.
#[tokio::test]
async fn the_agent_table_is_unchanged_by_a_login() {
    let mut rig = Rig::new(&[("FIXTURE_KEY", "set"), ("FIXTURE_CRED", CREDENTIAL)]).await;
    let columns = rig.agent_columns();
    let before = serde_json::to_vec(
        &rig.store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == rig.agent_id)
            .expect("the login row is registered")
            .agent,
    )
    .expect("a registry row serialises");

    rig.start_login().await;
    rig.harness.key("Enter");
    rig.until("finished", |frame| frame.contains("logged in:"))
        .await;

    let after = serde_json::to_vec(
        &rig.store
            .agents()
            .await
            .expect("the memory store never fails")
            .into_iter()
            .find(|summary| summary.agent.id == rig.agent_id)
            .expect("the login row is still registered")
            .agent,
    )
    .expect("a registry row serialises");
    assert_eq!(
        before, after,
        "byte for byte the row the login started from"
    );
    assert_eq!(
        rig.agent_columns(),
        columns,
        "and the registry's own columns are what they were"
    );
}

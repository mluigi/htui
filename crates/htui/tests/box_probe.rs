//! `StoreRequest::ProbeBox` through the shell (MOD-7 D11, D13; blueprint §6.6).
//!
//! **Fixture rule (blueprint H-7, F-T).** Nothing here probes this box: the runtime resolves
//! through an injected [`ProbeEnv`] whose `PATH` is an empty temporary directory and whose `home`
//! is `None`, the hardware is [`FixedHardware`], and every registry row is rewritten to a launch
//! whose one tool cannot exist, so tier 1 answers `missing` and nothing is spawned.
#![cfg(feature = "testkit")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use htui::agent_worker::AgentRuntime;
use htui::app::Action;
use htui::store_worker::StoreRequest;
use htui::testkit::Harness;
use htui_agent::box_probe::hardware::{FixedHardware, Hardware};
use htui_agent::probe::{ProbeEnv, platform_key};
use htui_agent::registry::DriverFactory;
use htui_core::store::{MemStore, WriteStore};
use serde_json::json;

/// The demo registry with every row's launch rewritten to resolve nowhere (H-7).
async fn unresolvable_registry() -> MemStore {
    let store = MemStore::demo();
    for summary in store.agents().await.expect("the memory store never fails") {
        let mut agent = summary.agent;
        agent.launch = json!({
            "command": "${gone}",
            "args": [],
            "env": {},
            "discovery": {
                "tools": {
                    "gone": { "kind": "path", "names": ["htui-no-such-binary-2f8e"] }
                },
                "handshake": true
            }
        });
        store.upsert_agent(&agent).await.expect("the row updates");
    }
    store
}

/// A box made of directories under `tmp`: `tmp/bin` is the whole `PATH`, `home` is `None`.
fn fake_env(tmp: &Path) -> ProbeEnv {
    std::fs::create_dir_all(tmp.join("bin")).expect("the fixture bin");
    let mut vars = BTreeMap::new();
    vars.insert(
        "PATH".to_owned(),
        tmp.join("bin").to_string_lossy().into_owned(),
    );
    ProbeEnv {
        cwd: tmp.to_path_buf(),
        platform: platform_key(),
        home: None,
        vars,
        versions: true,
        version_timeout: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn probe_box_through_the_shell_reports_on_the_status_line() {
    let tmp = tempfile::tempdir().expect("temp box");
    let runtime = AgentRuntime::new(DriverFactory::production()).with_probe_env(
        fake_env(tmp.path()),
        Arc::new(FixedHardware(Hardware {
            os_version: "Test OS 1".to_owned(),
            cpu: "Test CPU".to_owned(),
            ram_mb: Some(2048),
            display_vendors: vec!["0x1002".to_owned()],
        })),
    );
    let mut harness = Harness::over(unresolvable_registry().await).with_agent_runtime(runtime);
    harness.drive().await;

    harness.app().update(Action::Store(StoreRequest::ProbeBox));
    harness.drive_to_end().await;

    let status = harness.app().status.clone().unwrap_or_default();
    assert!(
        status.starts_with("box probed: "),
        "the report is on the status line: {status:?}"
    );
    assert!(status.contains("tags gpu"), "{status}");
}

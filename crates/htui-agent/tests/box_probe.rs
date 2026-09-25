//! The box probe (MOD-7 plan D6–D9, D17, D18; blueprint §5.7, D21–D24, D29, D36).
//!
//! Every tool case runs against scripts in a throwaway `bin` handed to the probe as its only
//! `PATH`, through an **injected** [`ProbeEnv`] (`home: None`): nothing here spawns a real host tool
//! (blueprint H-7). Hardware comes through [`FixedHardware`] or, for the GPU scan, a PCI device
//! tree built in a tempdir at run time — never a committed tree, because a real device directory is
//! named `0000:0f:00.0` and a committed `:` breaks a Windows checkout (blueprint F-J, D29). The one
//! exception is `this_box_reports_real_hardware`, which reads this box through `sysinfo` and spawns
//! nothing.
//!
//! The box-probe module keeps no `#[cfg(test)]` module of its own (D29), so every case lives here
//! and the tool-name grep over `src/box_probe/*.rs` is literal.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{TimeZone, Utc};
use htui_agent::box_probe::hardware::{
    FixedHardware, Hardware, HardwareSource, SystemHardware, pci_display_vendors,
    pnp_device_vendors, ram_mb, system_profiler_vendors,
};
use htui_agent::box_probe::spec::{
    EffectiveSpec, Fact, GpuVendor, SETTING_KEY, SPEC_IGNORED, Spec, TagRule, digest, effective,
    seed, validate,
};
// `tags_from_presence` is called by path: the blueprint names its test after it.
use htui_agent::box_probe::{self as box_probe, MAX_CONCURRENT_TOOLS, pick_gpu_vendor, probe_box};
use htui_agent::launch::{ToolProbe, VersionProbe, declares_install};
use htui_agent::probe::ProbeEnv;
use htui_core::model::{BoxId, BoxProbe};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------------------------

/// A box made of directories under `tmp`: `cwd`, and `bin` as the whole `PATH`. `home` is `None`,
/// so no `~` pattern can reach the maintainer's own home (blueprint F-T).
fn env(tmp: &Path) -> ProbeEnv {
    for dir in ["cwd", "bin"] {
        std::fs::create_dir_all(tmp.join(dir)).expect("fixture directory");
    }
    let mut vars = BTreeMap::new();
    vars.insert(
        "PATH".to_owned(),
        tmp.join("bin").to_string_lossy().into_owned(),
    );
    ProbeEnv {
        cwd: tmp.join("cwd"),
        platform: "linux-x86_64".to_owned(),
        home: None,
        vars,
        versions: true,
        version_timeout: Duration::from_secs(5),
    }
}

/// Writes an executable `sh` script named `name` into `tmp/bin`.
///
/// Every script of a case is written before that case's first probe: a `fork` elsewhere in the
/// process while a write handle is open would make `execve` answer `ETXTBSY` (`tests/probe.rs`'s
/// `executable` explains the window). The gate runs with `--test-threads=1`.
#[cfg(unix)]
fn script(tmp: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = tmp.join("bin").join(name);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

/// A fixed probe instant, so a whole `BoxProbe` compares equal.
fn now() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 25, 12, 0, 0)
        .single()
        .expect("a valid instant")
}

/// The seed with no stored overlay.
fn seeded() -> EffectiveSpec {
    effective(seed(), None)
}

/// Probes `env` under `spec` with no hardware facts.
async fn probe(env: &ProbeEnv, spec: &EffectiveSpec) -> BoxProbe {
    probe_box(
        BoxId::new(),
        env,
        &FixedHardware(Hardware::default()),
        spec,
        "0.1.0",
        now(),
    )
    .await
}

/// `(name, version)` of every tool the probe reported, in its order.
fn found(probe: &BoxProbe) -> Vec<(&str, &str)> {
    probe
        .tools
        .iter()
        .map(|tool| (tool.name.as_str(), tool.version.as_str()))
        .collect()
}

/// A PCI device directory `<root>/sys/bus/pci/devices/<slot>` holding `class` and `vendor` as the
/// kernel writes them (a trailing newline). `slot` uses `_` where the kernel uses `:` (D29).
fn pci_device(root: &Path, slot: &str, class: &str, vendor: &str) {
    let dir = root.join("sys/bus/pci/devices").join(slot);
    std::fs::create_dir_all(&dir).expect("device directory");
    std::fs::write(dir.join("class"), format!("{class}\n")).expect("class");
    std::fs::write(dir.join("vendor"), format!("{vendor}\n")).expect("vendor");
}

/// Asserts `stored` was refused: the seed is probed, and the sentence names `needle`.
fn assert_refused(stored: &Value, needle: &str) {
    let effective = effective(seed(), Some(stored));
    assert_eq!(effective.spec, *seed(), "a refused overlay probes the seed");
    assert_eq!(
        effective.digest,
        digest(seed()),
        "and records the seed's digest"
    );
    let error = effective.error.expect("a refusal carries a sentence");
    assert!(
        error.starts_with(&format!("{SPEC_IGNORED}: ")),
        "the sentence starts with the prefix: {error}"
    );
    assert!(error.contains(needle), "`{error}` names `{needle}`");
}

/// Asserts `stored` merged cleanly and returns the merged spec.
fn merged(stored: &Value) -> Spec {
    let effective = effective(seed(), Some(stored));
    assert_eq!(effective.error, None, "the overlay merges");
    assert_eq!(effective.digest, digest(&effective.spec));
    effective.spec
}

/// A `kind: path` probe with a version.
fn path_tool(name: &str, args: &[&str], pattern: &str) -> ToolProbe {
    ToolProbe::Path {
        names: vec![name.to_owned()],
        version: Some(VersionProbe {
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            pattern: pattern.to_owned(),
            min: None,
        }),
    }
}

// ---------------------------------------------------------------------------------------------
// The seed
// ---------------------------------------------------------------------------------------------

#[test]
fn the_spec_parses_and_every_tag_rule_names_a_listed_tool() {
    let spec = seed();
    assert_eq!(validate(spec), Ok(()));
    for rule in &spec.tags {
        for tool in &rule.any_tool {
            assert!(
                spec.tools.contains_key(tool),
                "rule `{}` names `{tool}`, which the seed does not list",
                rule.tag
            );
        }
    }
    assert_eq!(SETTING_KEY, "box_probe_spec");
}

#[test]
fn the_seed_lists_thirty_nine_path_tools() {
    let spec = seed();
    assert_eq!(spec.tools.len(), 39);
    for (name, probe) in &spec.tools {
        let ToolProbe::Path { names, .. } = probe else {
            panic!("`{name}` is not a `kind: path` tool");
        };
        assert!(!names.is_empty(), "`{name}` names no file");
        for file in names {
            assert!(
                !file.is_empty()
                    && file != "."
                    && file != ".."
                    && !file.contains('/')
                    && !file.contains('\\'),
                "`{name}` names `{file}`, which is not a bare file name"
            );
        }
    }
    // D9: `gradle` starts a JVM and a wrapper downloads a distribution; `bazel` is usually
    // `bazelisk`, whose first `--version` downloads a release. A probe must not touch the network.
    assert!(!spec.tools.contains_key("gradle"));
    assert!(!spec.tools.contains_key("bazel"));
    assert_eq!(spec.tags.len(), 14);
    assert_eq!(spec.gpu_vendors.len(), 5);
}

#[test]
fn the_derived_vocabulary_is_r_box_3_without_heavy_build_plus_oq12() {
    // `R-BOX-3`'s seeded tags as `htui-store` seeds them (`SEEDED_TAGS`, `pg/mod.rs:27-38`, a
    // private const `htui-agent` cannot see), minus `heavy_build`, which is declared only; plus
    // the five OQ-12 adds (`go`, `node`, `python`, `java`, `dotnet`). Written out on purpose: a
    // shared const would couple this crate to `htui-store`.
    let expected: BTreeSet<&str> = [
        "gpu", "vulkan", "msvc", "mingw", "clang", "cmake", "vcpkg", "docker",
        "rust", // R-BOX-3
        "go", "node", "python", "java", "dotnet", // OQ-12
    ]
    .into_iter()
    .collect();
    let derived: BTreeSet<&str> = seed().tags.iter().map(|rule| rule.tag.as_str()).collect();
    assert_eq!(derived, expected);
}

#[test]
fn tags_from_presence() {
    let rules = &seed().tags;
    let set = |names: &[&str]| -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    };
    let mingw =
        |answer: bool| -> BTreeMap<String, bool> { BTreeMap::from([("mingw".to_owned(), answer)]) };
    let none = BTreeMap::new();

    let cases: Vec<(
        &str,
        BTreeSet<String>,
        bool,
        BTreeMap<String, bool>,
        Vec<&str>,
    )> = vec![
        (
            "cargo alone",
            set(&["cargo"]),
            false,
            none.clone(),
            vec!["rust"],
        ),
        (
            "podman",
            set(&["podman"]),
            false,
            none.clone(),
            vec!["docker"],
        ),
        (
            "a mingw gcc",
            set(&["gcc"]),
            false,
            mingw(true),
            vec!["mingw"],
        ),
        ("a linux gcc", set(&["gcc"]), false, mingw(false), vec![]),
        (
            "a gcc never asked",
            set(&["gcc"]),
            false,
            none.clone(),
            vec![],
        ),
        ("a gpu", set(&[]), true, none.clone(), vec!["gpu"]),
        (
            "vulkaninfo",
            set(&["vulkaninfo"]),
            false,
            none.clone(),
            vec!["vulkan"],
        ),
        ("go", set(&["go"]), false, none.clone(), vec!["go"]),
        ("node", set(&["node"]), false, none.clone(), vec!["node"]),
        (
            "python3",
            set(&["python3"]),
            false,
            none.clone(),
            vec!["python"],
        ),
        ("javac", set(&["javac"]), false, none.clone(), vec!["java"]),
        ("java alone", set(&["java"]), false, none.clone(), vec![]),
        (
            "dotnet",
            set(&["dotnet"]),
            false,
            none.clone(),
            vec!["dotnet"],
        ),
        ("nothing", set(&[]), false, none.clone(), vec![]),
        (
            "sorted and deduplicated",
            set(&["rustc", "cargo", "podman", "docker", "cmake"]),
            true,
            none.clone(),
            vec!["cmake", "docker", "gpu", "rust"],
        ),
    ];
    for (what, present, gpu, asked, expected) in cases {
        assert_eq!(
            box_probe::tags_from_presence(rules, &present, gpu, &asked),
            expected,
            "{what}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Tools over a fake `PATH`
// ---------------------------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test]
async fn the_box_probe_forces_versions_on() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut env = env(tmp.path());
    env.versions = false;
    script(tmp.path(), "git", "echo 'git version 2.43.0'");

    let probe = probe(&env, &seeded()).await;
    assert_eq!(found(&probe), vec![("git", "2.43.0")]);
}

#[cfg(unix)]
#[tokio::test]
async fn tools_over_a_fake_path_report_versions_and_skip_the_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let env = env(tmp.path());
    let bin = tmp.path().join("bin");
    script(
        tmp.path(),
        "rustc",
        "echo 'rustc 1.98.1 (a1b2c3d4e 2026-08-01)'",
    );
    script(
        tmp.path(),
        "cmake",
        "echo 'cmake version 4.2.2'; echo; echo 'CMake suite maintained'",
    );
    script(
        tmp.path(),
        "docker",
        "echo 'Docker version 29.8.1, build x'",
    );
    script(tmp.path(), "go", "echo 'go version go1.25.7 linux/amd64'");
    script(tmp.path(), "node", "echo v22.19.0");

    let spec = seeded();
    let probe = probe(&env, &spec).await;
    assert_eq!(
        found(&probe),
        vec![
            ("cmake", "4.2.2"),
            ("docker", "29.8.1"),
            ("go", "1.25.7"),
            ("node", "22.19.0"),
            ("rustc", "1.98.1"),
        ],
        "every other seed tool is absent"
    );
    for tool in &probe.tools {
        assert_eq!(
            PathBuf::from(&tool.path),
            bin.join(&tool.name),
            "the path `which` resolved"
        );
    }
    assert_eq!(
        probe.probed_tags,
        vec!["cmake", "docker", "go", "node", "rust"]
    );
    assert_eq!(probe.spec_digest, spec.digest);
    assert_eq!(probe.htui_version, "0.1.0");
    assert_eq!(probe.probed_at, now());
}

#[cfg(unix)]
#[tokio::test]
async fn an_ask_runs_the_resolved_tool() {
    for (machine, mingw) in [("x86_64-w64-mingw32", true), ("x86_64-linux-gnu", false)] {
        let tmp = tempfile::tempdir().expect("tempdir");
        let env = env(tmp.path());
        script(
            tmp.path(),
            "gcc",
            &format!(
                "if [ \"$1\" = -dumpmachine ]; then echo {machine}; else echo 'gcc (GCC) 13.2.0'; fi"
            ),
        );

        let probe = probe(&env, &seeded()).await;
        assert_eq!(found(&probe), vec![("gcc", "13.2.0")], "{machine}");
        assert_eq!(
            probe.probed_tags.contains(&"mingw".to_owned()),
            mingw,
            "{machine}: {:?}",
            probe.probed_tags
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_hanging_tool_is_bounded_and_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut env = env(tmp.path());
    env.version_timeout = Duration::from_millis(200);
    script(tmp.path(), "cmake", "sleep 30");

    let started = Instant::now();
    let probe = probe(&env, &seeded()).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "bounded: {:?}",
        started.elapsed()
    );
    assert!(probe.tools.is_empty(), "a hung version is absent (OQ-11)");
    assert!(probe.probed_tags.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn a_shim_that_prints_no_version_is_absent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let env = env(tmp.path());
    script(
        tmp.path(),
        "pnpm",
        "echo 'Volta error: Could not find executable \"pnpm\"' >&2; exit 126",
    );

    let probe = probe(&env, &seeded()).await;
    assert!(probe.tools.is_empty(), "{:?}", probe.tools);
}

#[cfg(unix)]
#[tokio::test]
async fn a_presence_only_tool_counts_when_found() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let env = env(tmp.path());
    script(tmp.path(), "vulkaninfo", "exit 0");

    let probe = probe(&env, &seeded()).await;
    assert_eq!(found(&probe), vec![("vulkaninfo", "")]);
    assert_eq!(probe.probed_tags, vec!["vulkan"]);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn at_most_eight_tools_resolve_at_once() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let env = env(tmp.path());
    let log = tmp.path().join("log");
    let mut tools = serde_json::Map::new();
    for index in 0..16 {
        let name = format!("t{index:02}");
        script(
            tmp.path(),
            &name,
            &format!(
                "echo s >> '{log}'; sleep 0.3; echo e >> '{log}'; echo 1.0",
                log = log.display()
            ),
        );
        tools.insert(
            name.clone(),
            json!({"kind": "path", "names": [name], "version": {"args": [], "pattern": "^(\\d+\\.\\d+)$"}}),
        );
    }
    let spec = effective(seed(), Some(&json!({ "tools": tools })));
    assert_eq!(spec.error, None);

    let probe = probe(&env, &spec).await;
    assert_eq!(probe.tools.len(), 16, "{:?}", found(&probe));

    let text = std::fs::read_to_string(&log).expect("the log");
    let (mut running, mut most) = (0_i32, 0_i32);
    for line in text.lines() {
        match line {
            "s" => running += 1,
            "e" => running -= 1,
            other => panic!("unexpected log line `{other}`"),
        }
        most = most.max(running);
    }
    assert_eq!(MAX_CONCURRENT_TOOLS, 8);
    assert!(most <= 8, "at most eight at once, saw {most}");
    assert!(most >= 2, "and concurrently, saw {most}");
}

// ---------------------------------------------------------------------------------------------
// The GPU
// ---------------------------------------------------------------------------------------------

#[test]
fn linux_gpu_from_a_pci_fixture_root() {
    let tmp = tempfile::tempdir().expect("tempdir");
    pci_device(tmp.path(), "0000_00_01.0", "0x060400", "0x1022"); // a PCI bridge
    pci_device(tmp.path(), "0000_0f_00.0", "0x030000", "0x1002"); // a Radeon
    pci_device(tmp.path(), "0000_0f_00.1", "0x040300", "0x1002"); // its HDMI audio

    let vendors = pci_display_vendors(tmp.path());
    assert_eq!(vendors, vec!["0x1002"]);
    assert_eq!(pick_gpu_vendor(&vendors, &seed().gpu_vendors), Some("amd"));
}

#[test]
fn a_virtual_display_adapter_is_not_a_gpu() {
    let tmp = tempfile::tempdir().expect("tempdir");
    pci_device(tmp.path(), "0000_00_02.0", "0x030000", "0x1234"); // QEMU's stdvga

    let vendors = pci_display_vendors(tmp.path());
    assert_eq!(vendors, vec!["0x1234"]);
    assert_eq!(pick_gpu_vendor(&vendors, &seed().gpu_vendors), None);
}

#[test]
fn the_vendor_map_prefers_a_discrete_vendor() {
    let tmp = tempfile::tempdir().expect("tempdir");
    pci_device(tmp.path(), "0000_00_02.0", "0x030000", "0x8086");
    pci_device(tmp.path(), "0000_01_00.0", "0x030200", "0x10DE");

    let vendors = pci_display_vendors(tmp.path());
    assert_eq!(
        vendors.iter().collect::<BTreeSet<_>>(),
        ["0x10de".to_owned(), "0x8086".to_owned()].iter().collect(),
        "lowercased"
    );
    assert_eq!(
        pick_gpu_vendor(&vendors, &seed().gpu_vendors),
        Some("nvidia")
    );
}

#[test]
fn a_missing_pci_tree_is_no_gpu_not_an_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    assert!(pci_display_vendors(tmp.path()).is_empty());
    assert!(pci_display_vendors(&tmp.path().join("nowhere")).is_empty());
}

/// `system_profiler SPDisplaysDataType` on an Intel MacBook Pro with a discrete NVIDIA GPU.
const SYSTEM_PROFILER: &str = "\
Graphics/Displays:

    NVIDIA GeForce GT 750M:

      Chipset Model: NVIDIA GeForce GT 750M
      Type: GPU
      Bus: PCIe
      PCIe Lane Width: x8
      VRAM (Total): 2 GB
      Vendor: NVIDIA (0x10de)
      Device ID: 0x0fe9
      Revision ID: 0x00a2
      ROM Revision: 3776
      Automatic Graphics Switching: Supported
      gMux Version: 4.0.20 [3.2.8]
      Metal Family: Supported, Metal GPUFamily macOS 1

    Intel Iris Pro:

      Chipset Model: Intel Iris Pro
      Type: GPU
      Bus: Built-In
      VRAM (Dynamic, Max): 1536 MB
      Vendor: Intel (0x8086)
      Device ID: 0x0d26
      Revision ID: 0x0008
      Automatic Graphics Switching: Supported
      gMux Version: 4.0.20 [3.2.8]
      Metal Family: Supported, Metal GPUFamily macOS 1
      Displays:
        Color LCD:
          Display Type: Built-In Retina LCD
          Resolution: 2880 x 1800 Retina
";

/// `Get-CimInstance Win32_VideoController | ForEach-Object { $_.PNPDeviceID }` on a desktop with
/// one GeForce and the Remote Desktop adapter.
const PNP_DEVICE_IDS: &str = "\
PCI\\VEN_10DE&DEV_2684&SUBSYS_16F310DE&REV_A1\\4&2B3F8A7E&0&0008\r
SWD\\REMOTEDISPLAYENUM\\RDPIDD_INDIRECTDISPLAY&SESSIONID_0001\r
";

#[test]
fn macos_and_windows_gpu_parsers() {
    assert_eq!(
        system_profiler_vendors(SYSTEM_PROFILER),
        vec!["0x10de", "0x8086"]
    );
    assert_eq!(pnp_device_vendors(PNP_DEVICE_IDS), vec!["0x10de"]);
    assert!(system_profiler_vendors("Vendor: sppci_vendor_Apple").is_empty());
    assert!(pnp_device_vendors("ROOT\\BasicDisplay\\0000").is_empty());
}

#[test]
fn ram_is_floored_to_mib_and_bounded() {
    assert_eq!(ram_mb(0), None);
    assert_eq!(ram_mb(64 * 1024 * 1024 * 1024 + 5), Some(65536));
    assert_eq!(ram_mb(u64::MAX), None);
}

#[tokio::test]
async fn probe_box_takes_its_facts_from_the_hardware_seam() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let env = env(tmp.path());
    let hardware = FixedHardware(Hardware {
        os_version: "Ubuntu 24.04".to_owned(),
        cpu: "AMD Ryzen 9 7950X 16-Core Processor".to_owned(),
        ram_mb: Some(63_421),
        display_vendors: vec!["0x10de".to_owned()],
    });
    let spec = seeded();
    let box_id = BoxId::new();

    let probe = probe_box(box_id, &env, &hardware, &spec, "0.4.2", now()).await;
    assert_eq!(
        probe,
        BoxProbe {
            box_id,
            os_version: "Ubuntu 24.04".to_owned(),
            cpu: "AMD Ryzen 9 7950X 16-Core Processor".to_owned(),
            ram_mb: Some(63_421),
            gpu_present: true,
            gpu_vendor: Some("nvidia".to_owned()),
            tools: Vec::new(),
            probed_tags: vec!["gpu".to_owned()],
            htui_version: "0.4.2".to_owned(),
            spec_digest: spec.digest.clone(),
            probed_at: now(),
        }
    );
}

// ---------------------------------------------------------------------------------------------
// `declares_install` (MOD-7 D13)
// ---------------------------------------------------------------------------------------------

#[test]
fn declares_install_reads_discovery_install() {
    let with = json!({
        "command": "${adapter}",
        "discovery": {
            "tools": {"adapter": {"kind": "glob", "patterns": ["%HTUI_AGENTS_ROOT%/x/*/x"]}},
            "install": {"source": "acp_registry", "id": "x", "tool": "adapter"}
        }
    });
    let without = json!({
        "command": "${adapter}",
        "discovery": {"tools": {"adapter": {"kind": "path", "names": ["x"]}}}
    });
    assert!(declares_install(&with));
    assert!(!declares_install(&without));
    assert!(!declares_install(&json!({"command": "x"})));
    assert!(
        !declares_install(&json!({"discovery": {"install": {}}})),
        "does not parse"
    );
    assert!(!declares_install(&json!(42)));
}

// ---------------------------------------------------------------------------------------------
// The stored overlay (D17, D23, D24)
// ---------------------------------------------------------------------------------------------

#[test]
fn no_stored_spec_is_the_seed() {
    let effective = effective(seed(), None);
    assert_eq!(effective.spec, *seed());
    assert_eq!(effective.error, None);
    assert_eq!(effective.digest, digest(seed()));
}

#[test]
fn a_stored_tool_is_added() {
    let spec = merged(&json!({"tools": {"terraform": {
        "kind": "path",
        "names": ["terraform"],
        "version": {"args": ["version"], "pattern": "^Terraform v(\\S+)$"}
    }}}));
    assert_eq!(
        spec.tools.get("terraform"),
        Some(&path_tool("terraform", &["version"], "^Terraform v(\\S+)$"))
    );
    assert_eq!(spec.tools.len(), 40);
    for (name, probe) in &seed().tools {
        assert_eq!(spec.tools.get(name), Some(probe), "`{name}` is kept");
    }
    assert_eq!(spec.tags, seed().tags);
    assert_eq!(spec.gpu_vendors, seed().gpu_vendors);
}

#[test]
fn a_stored_tool_replaces_the_seeded_one() {
    let spec = merged(&json!({"tools": {"zig": {
        "kind": "path",
        "names": ["zig"],
        "version": {"args": ["--version"], "pattern": "^(\\S+)$"}
    }}}));
    assert_eq!(
        spec.tools.get("zig"),
        Some(&path_tool("zig", &["--version"], "^(\\S+)$"))
    );
    assert_ne!(spec.tools.get("zig"), seed().tools.get("zig"));
    assert_eq!(spec.tools.len(), 39);
}

#[test]
fn a_disabled_tool_and_its_tag_rule_drop_out() {
    let spec = merged(&json!({"tools": {"cmake": {"disabled": true}}}));
    assert!(!spec.tools.contains_key("cmake"));
    assert!(
        spec.tags.iter().all(|rule| rule.tag != "cmake"),
        "its only tool is gone"
    );
    assert_eq!(spec.tags.len(), 13);

    let spec = merged(&json!({"tools": {"cargo": {"disabled": true}}}));
    assert!(!spec.tools.contains_key("cargo"));
    let rust = spec
        .tags
        .iter()
        .find(|rule| rule.tag == "rust")
        .expect("`rust` keeps `rustc`");
    assert_eq!(rust.any_tool, vec!["rustc"]);

    // Disabling a name that does not exist is a no-op, not a fault.
    assert_eq!(
        merged(&json!({"tools": {"nosuch": {"disabled": true}}})),
        *seed()
    );
}

#[test]
fn a_stored_tag_rule_replaces_in_place() {
    let at = seed()
        .tags
        .iter()
        .position(|rule| rule.tag == "docker")
        .expect("seeded");
    let spec = merged(&json!({"tags": [
        {"tag": "docker", "any_tool": ["podman"]},
        {"tag": "zig", "any_tool": ["zig"]}
    ]}));
    assert_eq!(
        spec.tags[at],
        TagRule {
            tag: "docker".to_owned(),
            any_tool: vec!["podman".to_owned()],
            fact: None,
            ask: None,
        }
    );
    assert_eq!(spec.tags.len(), 15, "`zig` is appended");
    assert_eq!(spec.tags.last().map(|rule| rule.tag.as_str()), Some("zig"));

    let spec = merged(&json!({"tags": [{"tag": "gpu", "disabled": true}]}));
    assert!(spec.tags.iter().all(|rule| rule.fact != Some(Fact::Gpu)));
}

#[test]
fn a_stored_vendor_replaces_by_pci_id_and_keeps_priority() {
    let spec = merged(&json!({"gpu_vendors": [
        {"pci": "0x1002", "name": "radeon"},
        {"pci": "0x1ed5", "name": "moore_threads"}
    ]}));
    assert_eq!(
        spec.gpu_vendors[1],
        GpuVendor {
            pci: "0x1002".to_owned(),
            name: "radeon".to_owned(),
        }
    );
    assert_eq!(spec.gpu_vendors.len(), 6);
    assert_eq!(
        spec.gpu_vendors.last().map(|vendor| vendor.pci.as_str()),
        Some("0x1ed5"),
        "a new vendor has the lowest priority"
    );

    let spec = merged(&json!({"gpu_vendors": [{"pci": "0x8086", "disabled": true}]}));
    assert!(spec.gpu_vendors.iter().all(|vendor| vendor.pci != "0x8086"));
}

#[test]
fn an_unparseable_overlay_falls_back_to_the_seed_with_a_sentence() {
    assert_refused(&json!(42), "not a JSON object");
    assert_refused(&json!({"tool": {}}), "`tool`");
    assert_refused(&json!({"tools": []}), "tools");
    assert_refused(
        &json!({"tags": [{"tag": "x", "any_tool": ["git"], "extra": 1}]}),
        "tags",
    );
    assert_refused(
        &json!({"gpu_vendors": [{"pci": "0x10DE", "name": "x"}]}),
        "0x10DE",
    );
}

#[test]
fn a_glob_or_node_package_tool_is_refused() {
    assert_refused(
        &json!({"tools": {"myglob": {"kind": "glob", "patterns": ["~/bin/x"]}}}),
        "myglob",
    );
    assert_refused(
        &json!({"tools": {"mypkg": {
            "kind": "node_package", "package": "x", "entry": "y.js", "pinned": "1.0.0"
        }}}),
        "mypkg",
    );
}

#[test]
fn a_name_with_a_path_separator_is_refused() {
    assert_refused(
        &json!({"tools": {"x": {"kind": "path", "names": ["bin/x"]}}}),
        "bin/x",
    );
    assert_refused(
        &json!({"tools": {"x": {"kind": "path", "names": ["bin\\x"]}}}),
        "bin\\x",
    );
    assert_refused(
        &json!({"tools": {"x": {"kind": "path", "names": [".."]}}}),
        "..",
    );
    assert_refused(
        &json!({"tools": {"x": {"kind": "path", "names": []}}}),
        "tools.x",
    );
    assert_refused(
        &json!({"tools": {"../x": {"kind": "path", "names": ["x"]}}}),
        "../x",
    );
}

#[test]
fn a_bad_version_pattern_is_refused() {
    assert_refused(
        &json!({"tools": {"x": {"kind": "path", "names": ["x"], "version": {"args": [], "pattern": "("}}}}),
        "tools.x.version.pattern",
    );
    assert_refused(
        &json!({"tags": [{"tag": "x", "any_tool": ["git"], "ask": {"args": [], "matches": "("}}]}),
        "tags.x",
    );
}

#[test]
fn a_tag_rule_naming_an_absent_tool_is_refused() {
    assert_refused(
        &json!({"tags": [{"tag": "t", "any_tool": ["nosuch"]}]}),
        "nosuch",
    );
    assert_refused(
        &json!({"tags": [{"tag": "t", "any_tool": ["git"], "fact": "gpu"}]}),
        "tags.t",
    );
    assert_refused(&json!({"tags": [{"tag": "t"}]}), "tags.t");
}

#[test]
fn the_digest_is_stable_and_changes_with_the_spec() {
    assert_eq!(digest(seed()), digest(seed()));
    assert_eq!(effective(seed(), None).digest, digest(seed()));

    let more = merged(&json!({"tools": {"terraform": {"kind": "path", "names": ["terraform"]}}}));
    assert_ne!(digest(&more), digest(seed()));

    let ignored = effective(seed(), Some(&json!(42)));
    assert!(ignored.error.is_some());
    assert_eq!(ignored.digest, digest(seed()));

    let hex = digest(seed());
    assert_eq!(hex.len(), 64);
    assert!(
        hex.chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)),
        "{hex}"
    );
}

// ---------------------------------------------------------------------------------------------
// This box
// ---------------------------------------------------------------------------------------------

#[cfg(target_os = "linux")]
#[tokio::test]
async fn this_box_reports_real_hardware() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut env = env(tmp.path());
    env.vars.insert("PATH".to_owned(), String::new());

    let hardware = SystemHardware::host().read(&env).await;
    assert!(!hardware.os_version.is_empty(), "an OS version");
    assert!(!hardware.cpu.is_empty(), "a CPU brand");
    assert!(hardware.ram_mb.is_some_and(|mb| mb > 0), "some RAM");
}

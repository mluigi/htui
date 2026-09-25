//! The hardware half of the box probe (MOD-7 plan D6, D7; blueprint D21, D22).
//!
//! [`HardwareSource`] is the seam: production reads the host through [`SystemHardware`]
//! (`sysinfo` for the OS, CPU and RAM, and a per-OS GPU scan), and every test hands back fixed
//! facts through [`FixedHardware`]. The seam reports **raw PCI vendor ids**, never vendor names:
//! the probe spec's `gpu_vendors` map names them, and a stored overlay may replace that map, so
//! the naming belongs to `probe_box`, which holds the spec (blueprint F-B).
//!
//! The three text scanners are public and pure so the macOS and Windows GPU parsers are tested on
//! every OS against captured text.

use std::path::{Path, PathBuf};
use std::pin::Pin;

use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

use crate::probe::ProbeEnv;

/// What the hardware seam reports: facts only, never an error (plan D6).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hardware {
    /// `box.os_version`; empty when unreadable.
    pub os_version: String,
    /// `box.cpu`; empty when unreadable.
    pub cpu: String,
    /// `box.ram_mb`.
    pub ram_mb: Option<i32>,
    /// Lowercase `0x`-prefixed PCI vendor ids of every display-class device (blueprint D21),
    /// deduplicated; the probe spec's `gpu_vendors` names them.
    pub display_vendors: Vec<String>,
}

/// The boxed future a [`HardwareSource`] answers with.
pub type HardwareFuture<'a> = Pin<Box<dyn Future<Output = Hardware> + Send + 'a>>;

/// The seam between the box probe and the host (plan D6, blueprint D22).
pub trait HardwareSource: Send + Sync + core::fmt::Debug {
    /// Reads the facts; `env` bounds any child (`cwd`, `version_timeout`).
    fn read<'a>(&'a self, env: &'a ProbeEnv) -> HardwareFuture<'a>;
}

/// Production: `sysinfo` for OS, CPU and RAM; GPU per OS (D7).
#[derive(Debug, Clone)]
pub struct SystemHardware {
    /// The root the Linux PCI scan starts from.
    pci_root: PathBuf,
}

impl SystemHardware {
    /// Scans `<pci_root>/sys/bus/pci/devices` on Linux.
    #[must_use]
    pub fn new(pci_root: PathBuf) -> Self {
        Self { pci_root }
    }

    /// `new("/")`.
    #[must_use]
    pub fn host() -> Self {
        Self::new(PathBuf::from("/"))
    }
}

impl HardwareSource for SystemHardware {
    fn read<'a>(&'a self, env: &'a ProbeEnv) -> HardwareFuture<'a> {
        Box::pin(async move {
            // `sysinfo` reads the OS synchronously: off the runtime. A panicked or cancelled
            // read is "unreadable", which is empty facts.
            let facts = tokio::task::spawn_blocking(system_facts)
                .await
                .unwrap_or_default();
            let display_vendors = gpu_vendor_ids(&self.pci_root, env).await;
            Hardware {
                display_vendors,
                ..facts
            }
        })
    }
}

/// Tests: hands back exactly these facts.
#[derive(Debug, Clone)]
pub struct FixedHardware(pub Hardware);

impl HardwareSource for FixedHardware {
    fn read<'a>(&'a self, _env: &'a ProbeEnv) -> HardwareFuture<'a> {
        Box::pin(std::future::ready(self.0.clone()))
    }
}

/// `total_memory()` bytes → MiB, floored; `None` at 0 or above `i32::MAX`.
#[must_use]
pub fn ram_mb(total_bytes: u64) -> Option<i32> {
    i32::try_from(total_bytes / (1024 * 1024))
        .ok()
        .filter(|mb| *mb > 0)
}

/// Linux: `<root>/sys/bus/pci/devices/*/{class,vendor}`, `class` starting `0x03`; a missing tree
/// is empty.
///
/// Devices are visited in directory-name order, so the answer does not depend on `readdir`'s.
/// The directory names themselves are never parsed: the kernel's are `0000:0f:00.0`, and a test
/// tree uses `0000_0f_00.0` (blueprint D29).
#[must_use]
pub fn pci_display_vendors(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root.join("sys/bus/pci/devices")) else {
        return Vec::new();
    };
    let mut devices: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    devices.sort();

    let mut vendors = Vec::new();
    for device in devices {
        let Some(class) = read_hex(&device.join("class")) else {
            continue;
        };
        if !class.starts_with("0x03") {
            continue;
        }
        if let Some(vendor) = read_hex(&device.join("vendor")).and_then(|v| pci_id(&v)) {
            push_unique(&mut vendors, vendor);
        }
    }
    vendors
}

/// macOS `system_profiler SPDisplaysDataType`: every `(0x....)` on a line containing `Vendor`.
#[must_use]
pub fn system_profiler_vendors(text: &str) -> Vec<String> {
    let mut vendors = Vec::new();
    for line in text.lines().filter(|line| line.contains("Vendor")) {
        for (at, _) in line.match_indices("(0x") {
            let rest = &line[at + 1..];
            if let Some(id) = rest.get(..6)
                && rest.get(6..).is_some_and(|tail| tail.starts_with(')'))
                && let Some(id) = pci_id(id)
            {
                push_unique(&mut vendors, id);
            }
        }
    }
    vendors
}

/// Windows CIM `PNPDeviceID` lines: `VEN_10DE` → `0x10de`.
#[must_use]
pub fn pnp_device_vendors(text: &str) -> Vec<String> {
    let mut vendors = Vec::new();
    for line in text.lines() {
        let upper = line.to_ascii_uppercase();
        for (at, _) in upper.match_indices("VEN_") {
            let digits = &upper[at + 4..];
            if let Some(id) = digits.get(..4).and_then(|hex| pci_id(&format!("0x{hex}"))) {
                push_unique(&mut vendors, id);
            }
        }
    }
    vendors
}

/// `text` as a PCI vendor id: `0x` and exactly four hex digits, lowercased.
fn pci_id(text: &str) -> Option<String> {
    let lower = text.trim().to_ascii_lowercase();
    let digits = lower.strip_prefix("0x")?;
    (digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_hexdigit())).then_some(lower)
}

/// A sysfs attribute file, trimmed and lowercased; `None` when unreadable.
fn read_hex(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_ascii_lowercase())
}

/// Appends `id` unless it is already there: first-seen order, deduplicated.
fn push_unique(vendors: &mut Vec<String>, id: String) {
    if !vendors.contains(&id) {
        vendors.push(id);
    }
}

/// OS version, CPU brand and total RAM through one `sysinfo` refresh (plan D6).
///
/// The OS-name associated functions read the OS on every call, so they run here too, inside the
/// same blocking closure.
fn system_facts() -> Hardware {
    let sys = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let cpu = sys
        .cpus()
        .first()
        .map(|cpu| cpu.brand().trim().to_owned())
        .unwrap_or_default();
    Hardware {
        os_version: os_version(),
        cpu,
        ram_mb: ram_mb(sys.total_memory()),
        display_vendors: Vec::new(),
    }
}

/// macOS: the long OS version (`macOS 15.4 Sequoia`). Elsewhere: name and version joined by a
/// space (`Ubuntu 24.04`, `Windows 11`). Empty falls back to the long version, then the kernel's.
fn os_version() -> String {
    let primary = if cfg!(target_os = "macos") {
        System::long_os_version().unwrap_or_default()
    } else {
        [System::name(), System::os_version()]
            .into_iter()
            .flatten()
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    [
        Some(primary),
        System::long_os_version(),
        System::kernel_version(),
    ]
    .into_iter()
    .flatten()
    .map(|text| text.trim().to_owned())
    .find(|text| !text.is_empty())
    .unwrap_or_default()
}

/// Linux: the PCI scan under `root`, off the runtime.
#[cfg(target_os = "linux")]
async fn gpu_vendor_ids(root: &Path, _env: &ProbeEnv) -> Vec<String> {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || pci_display_vendors(&root))
        .await
        .unwrap_or_default()
}

/// macOS: Apple silicon's GPU is Apple's (`0x106b`); an Intel Mac asks `system_profiler`.
#[cfg(target_os = "macos")]
async fn gpu_vendor_ids(_root: &Path, env: &ProbeEnv) -> Vec<String> {
    if cfg!(target_arch = "aarch64") {
        return vec!["0x106b".to_owned()];
    }
    // Absolute: `run_bounded` resolves a bare command on the *process* `PATH`, not `env`'s.
    let launch = crate::launch::ResolvedLaunch {
        command: "/usr/sbin/system_profiler".to_owned(),
        args: vec!["SPDisplaysDataType".to_owned()],
        env: std::collections::BTreeMap::new(),
    };
    crate::probe::run_bounded("system_profiler", &launch, env)
        .await
        .map(|out| system_profiler_vendors(&out.stdout))
        .unwrap_or_default()
}

/// Windows: every video controller's `PNPDeviceID`, through the in-box PowerShell.
#[cfg(windows)]
async fn gpu_vendor_ids(_root: &Path, env: &ProbeEnv) -> Vec<String> {
    // Absolute: `run_bounded` resolves a bare command on the *process* `PATH`, not `env`'s.
    let command = format!(
        r"{}\System32\WindowsPowerShell\v1.0\powershell.exe",
        env.var("SystemRoot").unwrap_or(r"C:\Windows")
    );
    let launch = crate::launch::ResolvedLaunch {
        command,
        args: [
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance -ClassName Win32_VideoController | ForEach-Object { $_.PNPDeviceID }",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        env: std::collections::BTreeMap::new(),
    };
    crate::probe::run_bounded("Win32_VideoController", &launch, env)
        .await
        .map(|out| pnp_device_vendors(&out.stdout))
        .unwrap_or_default()
}

/// Anywhere else: no GPU scan.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
async fn gpu_vendor_ids(_root: &Path, _env: &ProbeEnv) -> Vec<String> {
    Vec::new()
}

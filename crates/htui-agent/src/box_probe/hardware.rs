//! The hardware half of the box probe (MOD-7 plan D6, D7; blueprint D21, D22).
//!
//! Red: the types are final, the bodies arrive with the green commit.

use std::path::{Path, PathBuf};
use std::pin::Pin;

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
        let _ = (&self.pci_root, env);
        todo!("MOD-7 T3: SystemHardware::read")
    }
}

/// Tests: hands back exactly these facts.
#[derive(Debug, Clone)]
pub struct FixedHardware(pub Hardware);

impl HardwareSource for FixedHardware {
    fn read<'a>(&'a self, env: &'a ProbeEnv) -> HardwareFuture<'a> {
        let _ = env;
        todo!("MOD-7 T3: FixedHardware::read")
    }
}

/// `total_memory()` bytes → MiB, floored; `None` at 0 or above `i32::MAX`.
#[must_use]
pub fn ram_mb(total_bytes: u64) -> Option<i32> {
    let _ = total_bytes;
    todo!("MOD-7 T3: ram_mb")
}

/// Linux: `<root>/sys/bus/pci/devices/*/{class,vendor}`, `class` starting `0x03`; a missing tree
/// is empty.
#[must_use]
pub fn pci_display_vendors(root: &Path) -> Vec<String> {
    let _ = root;
    todo!("MOD-7 T3: pci_display_vendors")
}

/// macOS `system_profiler SPDisplaysDataType`: every `(0x....)` on a line containing `Vendor`.
#[must_use]
pub fn system_profiler_vendors(text: &str) -> Vec<String> {
    let _ = text;
    todo!("MOD-7 T3: system_profiler_vendors")
}

/// Windows CIM `PNPDeviceID` lines: `VEN_10DE` → `0x10de`.
#[must_use]
pub fn pnp_device_vendors(text: &str) -> Vec<String> {
    let _ = text;
    todo!("MOD-7 T3: pnp_device_vendors")
}

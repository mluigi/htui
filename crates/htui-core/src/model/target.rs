//! Execution targets (MOD-130 M1): where a run executes, named once and chosen per level.
//!
//! A row of `target` names a kind and a locator (names and addresses, never secrets). Its
//! status is derived at read time and never stored (D4).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::box_::{BoxKind, BoxRow};
use crate::model::ids::{BoxId, TargetId};
use crate::seed::LOCAL_TARGET_ID;
use crate::store::StoreError;
use crate::store::traits::{has_nul, json_has_nul, target_locator_refused, target_name_refused};

str_enum!(
    /// `target.kind` (MOD-130 M1, D1): all five are stored; only [`RUNNABLE_KINDS`] run in this
    /// build.
    TargetKind {
        /// The box that starts the run (D2): the one built-in target.
        Local => "local",
        /// A container on this box (MOD-44).
        Docker => "docker",
        /// A remote machine over ssh (MOD-131).
        Ssh => "ssh",
        /// A container on a remote machine over ssh (MOD-131).
        SshDocker => "ssh_docker",
        /// A machine that enrolled itself (MOD-47).
        Enrolled => "enrolled",
    }
);

str_enum!(
    /// A target's status (MOD-130 M1, D4): derived at read time, never stored.
    TargetStatus {
        /// It can take a run now.
        Ready => "ready",
        /// It has no bound box yet, or was never probed.
        Unprepared => "unprepared",
        /// Its last probe failed, or its host lacks what the kind needs.
        Unreachable => "unreachable",
        /// Its kind does not run in this build.
        Planned => "planned",
    }
);

/// The kinds this build can run (MOD-130 M1, D1). MOD-44, MOD-131 and MOD-47 extend it.
pub const RUNNABLE_KINDS: &[TargetKind] = &[TargetKind::Local];

/// The `app_setting` key of the Settings default target (MOD-130 M1, D3): its value is a target
/// id; an absent row means the local target.
pub const DEFAULT_TARGET_KEY: &str = "default_target";

/// The Settings default target (MOD-130 M1, D3, blueprint B11): the `app_setting` row
/// [`DEFAULT_TARGET_KEY`] parsed, with its compare-and-set token. An absent row means the local
/// target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultTarget {
    /// The target the row names.
    pub target: TargetId,
    /// `app_setting.updated_at`: the token of
    /// [`set_default_target`](crate::store::WriteStore::set_default_target) (P6).
    pub updated_at: DateTime<Utc>,
}

impl DefaultTarget {
    /// The `app_setting.value` that names `target` (MOD-130 M1, D3): a JSON string of the id.
    #[must_use]
    pub fn value(target: TargetId) -> Value {
        Value::String(target.to_string())
    }

    /// Parses a stored [`DEFAULT_TARGET_KEY`] row (MOD-130 M1, blueprint B11). A value that is
    /// not a JSON string holding a target id is a backend error naming the key: only
    /// [`set_default_target`](crate::store::WriteStore::set_default_target) writes the row.
    ///
    /// # Errors
    /// [`StoreError::Backend`] for a malformed value.
    pub fn parse(value: &Value, updated_at: DateTime<Utc>) -> Result<Self, StoreError> {
        default_target_value(value).map(|target| Self { target, updated_at })
    }
}

/// The target id a [`DEFAULT_TARGET_KEY`] value names: the one rule of [`DefaultTarget::parse`]
/// and [`default_target_in`], so their sentence cannot drift.
fn default_target_value(value: &Value) -> Result<TargetId, StoreError> {
    value
        .as_str()
        .and_then(|text| text.parse::<TargetId>().ok())
        .ok_or_else(|| {
            StoreError::Backend(format!(
                "app_setting `{DEFAULT_TARGET_KEY}` holds {value}, not a target id"
            ))
        })
}

str_enum!(
    /// `run.target_source` (MOD-130 M2, D7): the cascade level that answered.
    TargetSource {
        /// The item's own target.
        Item => "item",
        /// The target of the run's step graph (D11).
        Workflow => "workflow",
        /// The item's project row (D5).
        Project => "project",
        /// The scope project row of the run's workspace (D5, D6).
        Workspace => "workspace",
        /// The Settings default, `app_setting` [`DEFAULT_TARGET_KEY`] (D3).
        Settings => "settings",
        /// No level named one: [`LOCAL_TARGET_ID`] (D2).
        Default => "default",
    }
);

impl TargetSource {
    /// MOD-130 M2 R1 F1: the phrase a target refusal names this level with
    /// (`…; chosen by its project`). [`TargetSource::Default`] is the local target and cannot
    /// refuse in practice; should it, it is `the built-in default`.
    #[must_use]
    pub const fn chosen_by(self) -> &'static str {
        match self {
            Self::Item => "the item",
            Self::Workflow => "its workflow",
            Self::Project => "its project",
            Self::Workspace => "its workspace",
            Self::Settings => "the Settings default",
            Self::Default => "the built-in default",
        }
    }
}

/// The cascade's levels, each `None` = inherit (MOD-130 M2, D3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetLevels {
    /// `item.target_id`.
    pub item: Option<TargetId>,
    /// `step_graph.target_id` of the graph the run resolves (D11).
    pub workflow: Option<TargetId>,
    /// The item's `project.target_id`; `None` for an item filed on a scope row (D5).
    pub project: Option<TargetId>,
    /// The scope project row's `target_id` (D5, D6).
    pub workspace: Option<TargetId>,
    /// The Settings default target (D3).
    pub settings: Option<TargetId>,
}

/// What [`resolve_target`] answers and a run records (MOD-130 M2, D7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedTarget {
    /// `run.target_id`.
    pub target: TargetId,
    /// `run.target_source`: the level that answered.
    pub source: TargetSource,
}

/// First `Some` in item, workflow, project, workspace, settings order; none → [`LOCAL_TARGET_ID`]
/// with [`TargetSource::Default`] (MOD-130 M2, D2, D3).
#[must_use]
pub fn resolve_target(levels: &TargetLevels) -> ResolvedTarget {
    let TargetLevels {
        item,
        workflow,
        project,
        workspace,
        settings,
    } = *levels;
    [
        (item, TargetSource::Item),
        (workflow, TargetSource::Workflow),
        (project, TargetSource::Project),
        (workspace, TargetSource::Workspace),
        (settings, TargetSource::Settings),
    ]
    .into_iter()
    .find_map(|(level, source)| level.map(|target| ResolvedTarget { target, source }))
    .unwrap_or(ResolvedTarget {
        target: LOCAL_TARGET_ID,
        source: TargetSource::Default,
    })
}

/// The rows `target_binding` reads (MOD-130 M2, P4): the target, the box its `box_id` names, and
/// the machine that box runs on (its parent for a container, else the bound row itself).
#[derive(Debug, Clone, PartialEq)]
pub struct TargetBinding {
    /// The `target` row.
    pub target: Target,
    /// The `box` row of `target.box_id`.
    pub bound: Option<BoxRow>,
    /// The machine `bound` runs on: its parent when it is a container, else `bound` itself.
    pub host: Option<BoxRow>,
}

/// Why a run cannot start on a target (MOD-130 M2, D1, D10, D14). `Display` is the sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TargetRefusal {
    /// The target is retired (D14).
    #[error("target {name} is retired; choose another target or inherit")]
    Retired {
        /// `target.name`.
        name: String,
    },
    /// The target's kind does not run in this build (D1).
    #[error("target {name} is {kind}, which this build cannot run yet; {owner} brings it")]
    NotRunnable {
        /// `target.name`.
        name: String,
        /// `target.kind`.
        kind: TargetKind,
        /// The handoff item that brings the kind.
        owner: &'static str,
    },
    /// The target runs on a box that is neither this box nor a child of it (D10).
    #[error(
        "target {name} belongs to box {owner}; start the run there (MOD-43) or choose another target"
    )]
    BoundElsewhere {
        /// `target.name`.
        name: String,
        /// The machine the target runs on.
        owner: BoxId,
    },
    /// The target's derived status is not `ready` (D4).
    #[error("target {name} is {status}; probe it before starting a run on it")]
    NotReady {
        /// `target.name`.
        name: String,
        /// The derived status.
        status: TargetStatus,
    },
}

/// Why a run on `binding` cannot start on box `here`, or `None` (MOD-130 M2, D1, D10). First
/// match wins: retired; kind not in `runnable` (owner from [`TargetKind::owner`]); `host` (or
/// `bound` when it is a machine) is not `here`; [`Target::status_with`] is not `Ready`.
#[must_use]
pub fn run_start_refusal(
    binding: &TargetBinding,
    here: BoxId,
    runnable: &[TargetKind],
) -> Option<TargetRefusal> {
    let target = &binding.target;
    let name = || target.name.clone();
    if target.retired_at.is_some() {
        return Some(TargetRefusal::Retired { name: name() });
    }
    if !runnable.contains(&target.kind) {
        return Some(TargetRefusal::NotRunnable {
            name: name(),
            kind: target.kind,
            owner: target.kind.owner().unwrap_or("a later item"),
        });
    }
    let machine = binding.host.as_ref().or_else(|| {
        binding
            .bound
            .as_ref()
            .filter(|bound| bound.kind == BoxKind::Machine)
    });
    if let Some(machine) = machine.filter(|machine| machine.id != here) {
        return Some(TargetRefusal::BoundElsewhere {
            name: name(),
            owner: machine.id,
        });
    }
    match target.status_with(runnable, binding.bound.as_ref(), binding.host.as_ref()) {
        TargetStatus::Ready => None,
        status => Some(TargetRefusal::NotReady {
            name: name(),
            status,
        }),
    }
}

/// The box the work runs on (MOD-130 M2): `binding.target.box_id`, else `here` (local).
#[must_use]
pub fn runs_on(binding: &TargetBinding, here: BoxId) -> BoxId {
    binding.target.box_id.unwrap_or(here)
}

/// The box that drives a run on `binding` started on `here`, as `NewRun.executing_box_id`
/// (MOD-130 M3, D8, P1): `None` for `enrolled` (its own worker drives it) and whenever the work
/// runs on `here` (local), where `COALESCE(executing_box_id, target_box_id)` already names
/// `here`; else `Some(here)`.
#[must_use]
pub fn driver_of(binding: &TargetBinding, here: BoxId) -> Option<BoxId> {
    (binding.target.kind != TargetKind::Enrolled && runs_on(binding, here) != here).then_some(here)
}

/// The Settings level of the cascade (MOD-130 M2, P5, D3): the [`DEFAULT_TARGET_KEY`] entry of
/// the `app_setting` map, `None` when absent. Same rule as [`DefaultTarget::parse`].
///
/// # Errors
/// [`StoreError::Backend`] naming the key for a value that is not a JSON string holding a
/// target id.
pub fn default_target_in(app: &BTreeMap<String, Value>) -> Result<Option<TargetId>, StoreError> {
    app.get(DEFAULT_TARGET_KEY)
        .map(default_target_value)
        .transpose()
}

/// The longest target name, in chars (MOD-130 M1, P9).
const TARGET_NAME_MAX: usize = 64;

/// The fragments of a normalised locator key that read as a secret (MOD-130 M1, blueprint B6).
const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "apikey",
    "privatekey",
    "credential",
];

impl TargetKind {
    /// Whether this build runs the kind (MOD-130 M1, D1): it is in [`RUNNABLE_KINDS`].
    #[must_use]
    pub fn runnable(self) -> bool {
        RUNNABLE_KINDS.contains(&self)
    }

    /// The handoff item that makes the kind runnable (MOD-130 M1, D1); `None` for `local`.
    #[must_use]
    pub const fn owner(self) -> Option<&'static str> {
        match self {
            Self::Local => None,
            Self::Docker => Some("MOD-44"),
            Self::Ssh | Self::SshDocker => Some("MOD-131"),
            Self::Enrolled => Some("MOD-47"),
        }
    }
}

/// A row of `target` (MOD-130 M1, ANA-42 §7.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    /// `target.id`.
    pub id: TargetId,
    /// `target.name`: unique, 1-64 characters, stored as given.
    pub name: String,
    /// `target.kind`.
    pub kind: TargetKind,
    /// `target.locator` (`JSONB` object): names and addresses only, never values.
    pub locator: Value,
    /// `target.box_id`: the box the target runs on once bound; always `None` on `local` (D2).
    pub box_id: Option<BoxId>,
    /// `target.probed_at`: the last probe of the bound box through this target.
    pub probed_at: Option<DateTime<Utc>>,
    /// `target.probe_error`: why the last probe failed; `None` after a good one.
    pub probe_error: Option<String>,
    /// `target.retired_at`: a retired target is hidden from pickers and keeps its history (D14).
    pub retired_at: Option<DateTime<Utc>>,
    /// `target.created_at`.
    pub created_at: DateTime<Utc>,
    /// `target.updated_at`: the compare-and-set token of the target's editors (P6).
    pub updated_at: DateTime<Utc>,
}

impl Target {
    /// The status D4 derives with this build's [`RUNNABLE_KINDS`] (MOD-130 M1). `bound` is the
    /// row of `box_id`; `host` is the machine `bound` runs on (its parent, for a container).
    #[must_use]
    pub fn status(&self, bound: Option<&BoxRow>, host: Option<&BoxRow>) -> TargetStatus {
        self.status_with(RUNNABLE_KINDS, bound, host)
    }

    /// [`Target::status`] with an explicit runnable set (MOD-130 M1, D4), first match wins:
    /// not runnable is `Planned`; `local` is `Ready`; no bound box is `Unprepared`; a probe
    /// error is `Unreachable`; never probed is `Unprepared`; a `docker` or `ssh_docker` target
    /// whose `host` lacks the `docker` tag (probed or declared) is `Unreachable`; else `Ready`.
    #[must_use]
    pub fn status_with(
        &self,
        runnable: &[TargetKind],
        bound: Option<&BoxRow>,
        host: Option<&BoxRow>,
    ) -> TargetStatus {
        if !runnable.contains(&self.kind) {
            return TargetStatus::Planned;
        }
        if self.kind == TargetKind::Local {
            return TargetStatus::Ready;
        }
        if bound.is_none() {
            return TargetStatus::Unprepared;
        }
        if self.probe_error.is_some() {
            return TargetStatus::Unreachable;
        }
        if self.probed_at.is_none() {
            return TargetStatus::Unprepared;
        }
        if matches!(self.kind, TargetKind::Docker | TargetKind::SshDocker) {
            let has_docker = host.is_some_and(|host| {
                host.probed_tags
                    .iter()
                    .chain(&host.declared_tags)
                    .any(|tag| tag == "docker")
            });
            if !has_docker {
                return TargetStatus::Unreachable;
            }
        }
        TargetStatus::Ready
    }
}

/// A target to create (MOD-130 M1, P4): never `local`, and never bound to a box here, since only
/// [`attach_child_box`](crate::store::WriteStore::attach_child_box) binds one in M1.
#[derive(Debug, Clone, PartialEq)]
pub struct NewTarget {
    /// `target.id`, minted by the caller.
    pub id: TargetId,
    /// `target.name`: 1-64 characters, unique, stored as given (blueprint B4).
    pub name: String,
    /// `target.kind`: any but `local` (D2).
    pub kind: TargetKind,
    /// `target.locator`: a JSON object of names and addresses, never secrets.
    pub locator: Value,
}

/// An edit of a target (MOD-130 M1, P6): `None` keeps the column. The kind and the box are
/// never edited.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TargetEdit {
    /// A new `target.name`.
    pub name: Option<String>,
    /// A new `target.locator`.
    pub locator: Option<Value>,
}

/// A container box to register under a machine and bind to a `docker` or `ssh_docker` target
/// (MOD-130 M1, P4, P5, blueprint B10).
#[derive(Debug, Clone, PartialEq)]
pub struct NewChildBox {
    /// The child's `box.id`, minted by the caller.
    pub id: BoxId,
    /// `box.parent_box_id`: the machine the container runs on.
    pub parent: BoxId,
    /// The child's `box.hostname`.
    pub hostname: String,
    /// The target the child is bound to.
    pub target: TargetId,
}

/// What [`attach_child_box`](crate::store::WriteStore::attach_child_box) answers (MOD-130 M1):
/// the child box as stored and the target as bound.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachedBox {
    /// The child `box` row.
    pub child: BoxRow,
    /// The `target` row, its `box_id` the child's.
    pub target: Target,
}

/// Why `name` cannot be a target name (MOD-130 M1, P9, blueprint B4), or `None`: 1-64 chars, no
/// control character, and no leading or trailing whitespace. The name is stored as given.
#[must_use]
pub fn target_name_refusal(name: &str) -> Option<String> {
    let chars = name.chars().count();
    let refused = chars == 0
        || chars > TARGET_NAME_MAX
        || name.chars().any(char::is_control)
        || name != name.trim();
    refused.then(|| target_name_refused(name))
}

/// Why `locator` cannot be stored (MOD-130 M1, blueprint B4, B6), or `None`. First match wins:
/// a NUL anywhere; not an object; a key at any depth whose normalisation (ASCII-lowercased, with
/// `_`, `-` and `.` removed) contains `password`, `passwd`, `secret`, `token`, `apikey`,
/// `privatekey` or `credential`; a string value holding a URL with a password in its userinfo.
#[must_use]
pub fn locator_refusal(locator: &Value) -> Option<String> {
    if json_has_nul(locator) {
        return Some(has_nul("target.locator"));
    }
    if !locator.is_object() {
        return Some(target_locator_refused("it is not a JSON object"));
    }
    secret_in(locator, "").map(|why| target_locator_refused(&why))
}

/// The first secret-looking key or value under `value`, as a sentence naming its path.
fn secret_in(value: &Value, path: &str) -> Option<String> {
    let join = |key: &str| {
        if path.is_empty() {
            key.to_owned()
        } else {
            format!("{path}.{key}")
        }
    };
    match value {
        Value::Object(map) => map.iter().find_map(|(key, item)| {
            let at = join(key);
            if key_looks_secret(key) {
                return Some(format!("key `{at}` looks like a secret"));
            }
            secret_in(item, &at)
        }),
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, item)| secret_in(item, &join(&index.to_string()))),
        Value::String(text) if url_has_password(text) => {
            Some(format!("the value at `{path}` holds a password in a URL"))
        }
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

/// Blueprint B6's key rule.
fn key_looks_secret(key: &str) -> bool {
    let normal: String = key
        .chars()
        .filter(|c| !matches!(c, '_' | '-' | '.'))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    SECRET_KEY_FRAGMENTS
        .iter()
        .any(|fragment| normal.contains(fragment))
}

/// Whether any `scheme://user:pass@host` in `text` carries a password in its userinfo.
fn url_has_password(text: &str) -> bool {
    text.match_indices("://").any(|(at, sep)| {
        let rest = &text[at + sep.len()..];
        let authority = rest
            .split(|c: char| matches!(c, '/' | '?' | '#') || c.is_whitespace())
            .next()
            .unwrap_or_default();
        authority
            .rsplit_once('@')
            .is_some_and(|(userinfo, _)| userinfo.contains(':'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::box_::OsFamily;
    use crate::model::ids::UserId;
    use serde_json::json;

    fn at() -> DateTime<Utc> {
        DateTime::from_timestamp(1_788_393_600, 0).expect("a valid timestamp")
    }

    fn target(kind: TargetKind) -> Target {
        Target {
            id: TargetId::new(),
            name: format!("a {kind}"),
            kind,
            locator: json!({}),
            box_id: None,
            probed_at: None,
            probe_error: None,
            retired_at: None,
            created_at: at(),
            updated_at: at(),
        }
    }

    fn a_box(tags: &[&str], declared: &[&str]) -> BoxRow {
        BoxRow {
            id: BoxId::new(),
            user_id: UserId::new(),
            hostname: "dev".to_owned(),
            os_family: Some(OsFamily::Linux),
            os_version: String::new(),
            arch: String::new(),
            cpu: String::new(),
            ram_mb: None,
            gpu_present: false,
            gpu_vendor: None,
            htui_version: String::new(),
            probed_tags: tags.iter().map(|&t| t.to_owned()).collect(),
            declared_tags: declared.iter().map(|&t| t.to_owned()).collect(),
            quirks: String::new(),
            settings: json!({}),
            registered_at: at(),
            last_seen_at: at(),
            last_probed_at: None,
            updated_at: at(),
            edit_version: 0,
            kind: BoxKind::Machine,
            parent_box_id: None,
        }
    }

    /// D1: in this build only `local` runs, so every other kind reads `planned` whatever its box.
    #[test]
    fn every_kind_but_local_is_planned_in_this_build() {
        assert_eq!(RUNNABLE_KINDS, &[TargetKind::Local]);
        let host = a_box(&["docker"], &[]);
        for &kind in TargetKind::ALL {
            let mut row = target(kind);
            row.probed_at = Some(at());
            let want = if kind == TargetKind::Local {
                TargetStatus::Ready
            } else {
                TargetStatus::Planned
            };
            assert_eq!(row.status(None, None), want, "{kind}");
            assert_eq!(row.status(Some(&host), Some(&host)), want, "{kind}");
            assert_eq!(kind.runnable(), kind == TargetKind::Local, "{kind}");
        }
        assert_eq!(TargetKind::Local.owner(), None);
        assert_eq!(TargetKind::Docker.owner(), Some("MOD-44"));
        assert_eq!(TargetKind::Ssh.owner(), Some("MOD-131"));
        assert_eq!(TargetKind::SshDocker.owner(), Some("MOD-131"));
        assert_eq!(TargetKind::Enrolled.owner(), Some("MOD-47"));
    }

    /// D4's rules 1-7 in order, each one shadowing the rules below it.
    #[test]
    fn status_follows_the_d4_order() {
        let all = TargetKind::ALL;
        let bound = a_box(&[], &[]);
        let docker_host = a_box(&["docker"], &[]);
        let declared_host = a_box(&[], &["docker"]);
        let bare_host = a_box(&["linux"], &["gaming"]);

        // 1. Not runnable → planned, before anything else is read.
        let mut docker = target(TargetKind::Docker);
        docker.probed_at = Some(at());
        assert_eq!(
            docker.status_with(&[TargetKind::Local], Some(&bound), Some(&docker_host)),
            TargetStatus::Planned
        );
        // 2. Local → ready, whatever it carries.
        let mut local = target(TargetKind::Local);
        local.probe_error = Some("ignored".to_owned());
        assert_eq!(local.status_with(all, None, None), TargetStatus::Ready);
        // 3. No bound box → unprepared, even with a probe error.
        let mut unbound = target(TargetKind::Ssh);
        unbound.probe_error = Some("refused".to_owned());
        assert_eq!(
            unbound.status_with(all, None, None),
            TargetStatus::Unprepared
        );
        // 4. A probe error → unreachable, even when never probed.
        let mut failed = target(TargetKind::Ssh);
        failed.probe_error = Some("connection refused".to_owned());
        assert_eq!(
            failed.status_with(all, Some(&bound), Some(&bound)),
            TargetStatus::Unreachable
        );
        // 5. Never probed → unprepared.
        let fresh = target(TargetKind::Docker);
        assert_eq!(
            fresh.status_with(all, Some(&bound), Some(&docker_host)),
            TargetStatus::Unprepared
        );
        // 6. A docker or ssh_docker target whose host lacks the docker tag → unreachable.
        for kind in [TargetKind::Docker, TargetKind::SshDocker] {
            let mut row = target(kind);
            row.probed_at = Some(at());
            assert_eq!(
                row.status_with(all, Some(&bound), Some(&bare_host)),
                TargetStatus::Unreachable,
                "{kind}"
            );
            assert_eq!(
                row.status_with(all, Some(&bound), None),
                TargetStatus::Unreachable,
                "{kind} without a host"
            );
            // 7. With the tag, probed or declared → ready.
            assert_eq!(
                row.status_with(all, Some(&bound), Some(&docker_host)),
                TargetStatus::Ready,
                "{kind}"
            );
            assert_eq!(
                row.status_with(all, Some(&bound), Some(&declared_host)),
                TargetStatus::Ready,
                "{kind}"
            );
        }
        // 7. An ssh or enrolled target needs no docker tag.
        for kind in [TargetKind::Ssh, TargetKind::Enrolled] {
            let mut row = target(kind);
            row.probed_at = Some(at());
            assert_eq!(
                row.status_with(all, Some(&bound), Some(&bare_host)),
                TargetStatus::Ready,
                "{kind}"
            );
        }
    }

    /// Blueprint B6: any depth, normalised keys that contain a secret word, and a URL password.
    #[test]
    fn locator_refusal_names_a_secret_key_at_any_depth() {
        for (locator, path) in [
            (json!({"password": "x"}), "password"),
            (json!({"Token": "x"}), "Token"),
            (json!({"docker": {"API_KEY": "x"}}), "docker.API_KEY"),
            (
                json!({"docker": {"client_secret": "x"}}),
                "docker.client_secret",
            ),
            (json!({"hosts": [{"passwd": "x"}]}), "hosts.0.passwd"),
            (json!({"access_token": "x"}), "access_token"),
            (json!({"clientSecret": "x"}), "clientSecret"),
            (
                json!({"AWS_SECRET_ACCESS_KEY": "x"}),
                "AWS_SECRET_ACCESS_KEY",
            ),
            (json!({"private-key": "x"}), "private-key"),
            (json!({"privateKey": "x"}), "privateKey"),
            (json!({"ssh_password": "x"}), "ssh_password"),
            (json!({"credentials": {}}), "credentials"),
        ] {
            assert_eq!(
                locator_refusal(&locator),
                Some(target_locator_refused(&format!(
                    "key `{path}` looks like a secret"
                ))),
                "{locator}"
            );
        }
        assert_eq!(
            locator_refusal(&json!({"docker": {"host": "tcp://u:p@h:2375"}})),
            Some(target_locator_refused(
                "the value at `docker.host` holds a password in a URL"
            ))
        );
        assert_eq!(
            locator_refusal(&json!([])),
            Some(target_locator_refused("it is not a JSON object"))
        );
        assert_eq!(
            locator_refusal(&json!("dev")),
            Some(target_locator_refused("it is not a JSON object"))
        );
        assert_eq!(
            locator_refusal(&json!({"host": "de\u{0}v"})),
            Some(has_nul("target.locator"))
        );
        assert_eq!(
            locator_refusal(&json!({"ho\u{0}st": "dev"})),
            Some(has_nul("target.locator"))
        );
        for allowed in [
            json!({}),
            json!({"host": "dev", "user": "me", "context": "default"}),
            json!({"host": "ssh://me@dev:22", "docker": {"host": "tcp://[::1]:2375"}}),
            json!({"hosts": ["dev", "build"], "port": 22, "tls": true}),
        ] {
            assert_eq!(locator_refusal(&allowed), None, "{allowed}");
        }
    }

    /// Blueprint B4: a name is stored as given, so space around it is refused, not trimmed.
    #[test]
    fn a_name_is_bounded_and_never_trimmed() {
        for good in [
            "this box",
            "a",
            "docker: rust 1.95",
            &"x".repeat(64),
            &"é".repeat(64),
        ] {
            assert_eq!(target_name_refusal(good), None, "{good:?}");
        }
        for bad in [
            "",
            " ",
            " dev",
            "dev ",
            "\tdev",
            "de\nv",
            "de\u{0}v",
            "de\u{7f}v",
            &"x".repeat(65),
        ] {
            assert_eq!(
                target_name_refusal(bad),
                Some(target_name_refused(bad)),
                "{bad:?}"
            );
        }
    }

    /// `chk_target_kind`'s list in `0040_execution_targets.sql`, in order; the status texts are
    /// pinned too, as the UI and the cache read them.
    #[test]
    fn kind_and_status_match_the_check_list() {
        let kinds: Vec<_> = TargetKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds, ["local", "docker", "ssh", "ssh_docker", "enrolled"]);
        let statuses: Vec<_> = TargetStatus::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(statuses, ["ready", "unprepared", "unreachable", "planned"]);
        for &kind in TargetKind::ALL {
            assert_eq!(kind.as_str().parse::<TargetKind>(), Ok(kind));
            assert_eq!(
                serde_json::to_value(kind).expect("a kind serialises"),
                json!(kind.as_str())
            );
        }
        for &status in TargetStatus::ALL {
            assert_eq!(status.as_str().parse::<TargetStatus>(), Ok(status));
        }
    }

    fn binding(target: Target, bound: Option<BoxRow>, host: Option<BoxRow>) -> TargetBinding {
        TargetBinding {
            target,
            bound,
            host,
        }
    }

    /// `chk_run_target_source`'s list in `0040_execution_targets.sql`, in order (MOD-130 M2, D7).
    #[test]
    fn target_source_matches_the_check_list() {
        let sources: Vec<_> = TargetSource::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(
            sources,
            [
                "item",
                "workflow",
                "project",
                "workspace",
                "settings",
                "default"
            ]
        );
        for &source in TargetSource::ALL {
            assert_eq!(source.as_str().parse::<TargetSource>(), Ok(source));
            assert_eq!(
                serde_json::to_value(source).expect("a source serialises"),
                json!(source.as_str())
            );
            assert_eq!(
                serde_json::from_value::<TargetSource>(json!(source.as_str()))
                    .expect("a source deserialises"),
                source
            );
        }
    }

    /// MOD-130 M2 R1 F1: the phrase a target refusal names its level with, one per source.
    #[test]
    fn chosen_by_names_every_level() {
        let phrases: Vec<_> = TargetSource::ALL
            .iter()
            .map(|source| (source.as_str(), source.chosen_by()))
            .collect();
        assert_eq!(
            phrases,
            [
                ("item", "the item"),
                ("workflow", "its workflow"),
                ("project", "its project"),
                ("workspace", "its workspace"),
                ("settings", "the Settings default"),
                ("default", "the built-in default"),
            ]
        );
    }

    /// D3: all 32 combinations; the first level set wins, none is the local target (D2).
    #[test]
    fn resolve_target_takes_the_first_level_that_names_one() {
        let ids: [TargetId; 5] = std::array::from_fn(|_| TargetId::new());
        let sources = [
            TargetSource::Item,
            TargetSource::Workflow,
            TargetSource::Project,
            TargetSource::Workspace,
            TargetSource::Settings,
        ];
        for mask in 0u8..32 {
            let set = |level: usize| (mask & (1 << level) != 0).then_some(ids[level]);
            let levels = TargetLevels {
                item: set(0),
                workflow: set(1),
                project: set(2),
                workspace: set(3),
                settings: set(4),
            };
            let want = (0..5).find(|&level| mask & (1 << level) != 0).map_or(
                ResolvedTarget {
                    target: LOCAL_TARGET_ID,
                    source: TargetSource::Default,
                },
                |level| ResolvedTarget {
                    target: ids[level],
                    source: sources[level],
                },
            );
            assert_eq!(resolve_target(&levels), want, "mask {mask:05b}");
        }
    }

    /// MOD-130 M2: retired, then not runnable, then bound elsewhere, then not ready.
    #[test]
    fn run_start_refusal_follows_its_order() {
        let here_box = a_box(&["docker"], &[]);
        let here = here_box.id;
        let all = TargetKind::ALL;

        // Retired comes first, even on local.
        let mut retired = target(TargetKind::Local);
        retired.retired_at = Some(at());
        assert_eq!(
            run_start_refusal(&binding(retired.clone(), None, None), here, RUNNABLE_KINDS),
            Some(TargetRefusal::Retired {
                name: retired.name.clone()
            })
        );
        let mut retired_docker = target(TargetKind::Docker);
        retired_docker.retired_at = Some(at());
        assert_eq!(
            run_start_refusal(&binding(retired_docker.clone(), None, None), here, &[]),
            Some(TargetRefusal::Retired {
                name: retired_docker.name.clone()
            })
        );

        // Not runnable names the item that brings the kind.
        for (kind, owner) in [
            (TargetKind::Docker, "MOD-44"),
            (TargetKind::Ssh, "MOD-131"),
            (TargetKind::SshDocker, "MOD-131"),
            (TargetKind::Enrolled, "MOD-47"),
        ] {
            let row = target(kind);
            assert_eq!(
                run_start_refusal(
                    &binding(row.clone(), Some(here_box.clone()), Some(here_box.clone())),
                    here,
                    RUNNABLE_KINDS
                ),
                Some(TargetRefusal::NotRunnable {
                    name: row.name.clone(),
                    kind,
                    owner
                }),
                "{kind}"
            );
        }
        // Local outside the runnable set has no owner and never panics.
        let local = target(TargetKind::Local);
        assert_eq!(
            run_start_refusal(&binding(local.clone(), None, None), here, &[]),
            Some(TargetRefusal::NotRunnable {
                name: local.name.clone(),
                kind: TargetKind::Local,
                owner: "a later item"
            })
        );

        // A container whose machine is another box.
        let elsewhere = a_box(&["docker"], &[]);
        let mut child_elsewhere = a_box(&[], &[]);
        child_elsewhere.kind = BoxKind::Container;
        child_elsewhere.parent_box_id = Some(elsewhere.id);
        let mut docker = target(TargetKind::Docker);
        docker.box_id = Some(child_elsewhere.id);
        docker.probed_at = Some(at());
        assert_eq!(
            run_start_refusal(
                &binding(
                    docker.clone(),
                    Some(child_elsewhere.clone()),
                    Some(elsewhere.clone())
                ),
                here,
                all
            ),
            Some(TargetRefusal::BoundElsewhere {
                name: docker.name.clone(),
                owner: elsewhere.id
            })
        );
        // An ssh target bound to a machine that is not here, with no host row read.
        let mut ssh = target(TargetKind::Ssh);
        ssh.box_id = Some(elsewhere.id);
        ssh.probed_at = Some(at());
        assert_eq!(
            run_start_refusal(
                &binding(ssh.clone(), Some(elsewhere.clone()), None),
                here,
                all
            ),
            Some(TargetRefusal::BoundElsewhere {
                name: ssh.name.clone(),
                owner: elsewhere.id
            })
        );
        assert_eq!(
            run_start_refusal(
                &binding(
                    ssh.clone(),
                    Some(elsewhere.clone()),
                    Some(elsewhere.clone())
                ),
                here,
                all
            ),
            Some(TargetRefusal::BoundElsewhere {
                name: ssh.name.clone(),
                owner: elsewhere.id
            })
        );

        // A container on a child of here: unprobed is not ready; probed with docker runs.
        let mut child_here = a_box(&[], &[]);
        child_here.kind = BoxKind::Container;
        child_here.parent_box_id = Some(here);
        let mut fresh = target(TargetKind::Docker);
        fresh.box_id = Some(child_here.id);
        assert_eq!(
            run_start_refusal(
                &binding(
                    fresh.clone(),
                    Some(child_here.clone()),
                    Some(here_box.clone())
                ),
                here,
                all
            ),
            Some(TargetRefusal::NotReady {
                name: fresh.name.clone(),
                status: TargetStatus::Unprepared
            })
        );
        let mut probed = fresh.clone();
        probed.probed_at = Some(at());
        assert_eq!(
            run_start_refusal(
                &binding(probed, Some(child_here.clone()), Some(here_box.clone())),
                here,
                all
            ),
            None
        );

        // Local runs.
        assert_eq!(
            run_start_refusal(
                &binding(target(TargetKind::Local), None, None),
                here,
                RUNNABLE_KINDS
            ),
            None
        );
    }

    /// The sentences are the contract (MOD-130 M2, P6): pinned verbatim.
    #[test]
    fn refusal_sentences_are_pinned() {
        let owner = BoxId::from_uuid(uuid::Uuid::from_u128(
            0x0000_0000_0000_4000_8000_0000_0000_00b0,
        ));
        for (refusal, want) in [
            (
                TargetRefusal::Retired {
                    name: "old box".to_owned(),
                },
                "target old box is retired; choose another target or inherit",
            ),
            (
                TargetRefusal::NotRunnable {
                    name: "rust".to_owned(),
                    kind: TargetKind::Docker,
                    owner: "MOD-44",
                },
                "target rust is docker, which this build cannot run yet; MOD-44 brings it",
            ),
            (
                TargetRefusal::NotRunnable {
                    name: "far".to_owned(),
                    kind: TargetKind::SshDocker,
                    owner: "MOD-131",
                },
                "target far is ssh_docker, which this build cannot run yet; MOD-131 brings it",
            ),
            (
                TargetRefusal::BoundElsewhere {
                    name: "rust".to_owned(),
                    owner,
                },
                "target rust belongs to box 00000000-0000-4000-8000-0000000000b0; start the run \
                 there (MOD-43) or choose another target",
            ),
            (
                TargetRefusal::NotReady {
                    name: "rust".to_owned(),
                    status: TargetStatus::Unprepared,
                },
                "target rust is unprepared; probe it before starting a run on it",
            ),
            (
                TargetRefusal::NotReady {
                    name: "rust".to_owned(),
                    status: TargetStatus::Unreachable,
                },
                "target rust is unreachable; probe it before starting a run on it",
            ),
        ] {
            assert_eq!(refusal.to_string(), want);
        }
    }

    /// MOD-130 M2: a bound target runs on its box; local runs here.
    #[test]
    fn runs_on_is_the_bound_box_else_here() {
        let here = BoxId::new();
        assert_eq!(
            runs_on(&binding(target(TargetKind::Local), None, None), here),
            here
        );
        let child = a_box(&[], &[]);
        let mut docker = target(TargetKind::Docker);
        docker.box_id = Some(child.id);
        assert_eq!(
            runs_on(&binding(docker, Some(child.clone()), None), here),
            child.id
        );
    }

    /// MOD-130 M3, P1: only work that runs on another box than `here` names `here` as its driver;
    /// `enrolled` is driven by its own worker, and local work is driven by its target box.
    #[test]
    fn driver_of_names_here_only_for_work_elsewhere() {
        let here = BoxId::new();
        assert_eq!(
            driver_of(&binding(target(TargetKind::Local), None, None), here),
            None,
            "local"
        );

        let mut machine = a_box(&[], &[]);
        machine.id = here;
        let mut child = a_box(&[], &[]);
        child.kind = BoxKind::Container;
        child.parent_box_id = Some(here);
        let mut docker = target(TargetKind::Docker);
        docker.box_id = Some(child.id);
        assert_eq!(
            driver_of(
                &binding(docker, Some(child.clone()), Some(machine.clone())),
                here
            ),
            Some(here),
            "docker on a child of here"
        );

        let other = a_box(&[], &[]);
        let mut ssh = target(TargetKind::Ssh);
        ssh.box_id = Some(other.id);
        assert_eq!(
            driver_of(
                &binding(ssh.clone(), Some(other.clone()), Some(other.clone())),
                here
            ),
            Some(here),
            "ssh bound to another machine"
        );
        ssh.box_id = Some(here);
        assert_eq!(
            driver_of(
                &binding(ssh, Some(machine.clone()), Some(machine.clone())),
                here
            ),
            None,
            "ssh bound to here itself"
        );

        let mut enrolled = target(TargetKind::Enrolled);
        enrolled.box_id = Some(other.id);
        assert_eq!(
            driver_of(&binding(enrolled, Some(other.clone()), Some(other)), here),
            None,
            "enrolled on another machine"
        );
    }

    /// P5: the Settings level is read from the `app_setting` map with `DefaultTarget::parse`'s rule.
    #[test]
    fn default_target_in_reads_the_setting() {
        let mut app = BTreeMap::new();
        assert_eq!(default_target_in(&app), Ok(None));
        app.insert("other".to_owned(), json!(3));
        assert_eq!(default_target_in(&app), Ok(None));
        let id = TargetId::new();
        app.insert(DEFAULT_TARGET_KEY.to_owned(), DefaultTarget::value(id));
        assert_eq!(default_target_in(&app), Ok(Some(id)));
        for bad in [json!(3), json!("not an id"), json!(null)] {
            app.insert(DEFAULT_TARGET_KEY.to_owned(), bad.clone());
            let err = default_target_in(&app).expect_err("a malformed value is refused");
            let StoreError::Backend(text) = &err else {
                panic!("{bad}: {err:?}");
            };
            assert!(text.contains("default_target"), "{text}");
            assert_eq!(
                Some(err),
                DefaultTarget::parse(&bad, at()).err(),
                "the two readers share one sentence"
            );
        }
    }
}

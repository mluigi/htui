//! `htui provision <destination>` (MOD-45): installs this htui build as the `htui-worker` system
//! service on a Linux host over the user's own `ssh`, with the DSN encrypted on that host by
//! `systemd-creds` and nowhere else (`R-STO-1`, PRD D1–D3).
//!
//! Four short sessions: the preflight (read-only), prepare (unprivileged: the log directory and the
//! binary), install (privileged: the credential, the unit, the start) and verify (wait for the
//! service and its `box.toml`). Then, from this machine, the box is checked in Postgres against a
//! baseline taken before the install, and its executor is set to `worker` (D313).
//!
//! Every remote script is a constant in [`script`]. Values reach a script only as quoted
//! positional arguments. The DSN and the sudo password travel only on the ssh child's stdin:
//! never in an `argv`, the environment, a log or a file on either side.
//!
//! A refusal (exit 2) is anything decided before the first remote write. A failure (exit 1) is
//! anything after it. `remote::SshRemote` is the only place `htui` spawns `ssh`.

pub mod plan;
pub mod preflight;
pub mod script;

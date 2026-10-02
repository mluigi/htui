//! MOD-24 D1: where a crash test may stop a worker, at the walk's seams no test part reaches.
//!
//! [`reached`] is called at three sites. [`KillPoint::Documented`] is in the engine's single-step
//! walk, after the session sink wrote the step's output document and before verify.
//! [`KillPoint::Captured`] is in the same walk, after the step's `after` commits are recorded and
//! before its output is read and the step finished. [`KillPoint::CommandPicked`] is in
//! `htui-worker`'s command poll, once a pending row is this process's to apply and before its
//! cancel is spawned. Fan-out candidates and judges have none (plan D1).
//!
//! Without this crate's `test-support` feature, [`reached`] is an empty `#[inline]` function and
//! nothing reads the environment. The feature is never enabled outside test builds: a plain or
//! release `cargo build` of `htui` leaves it off. A `cargo test` or `--all-targets` build of the
//! `htui` binary does compile it in (the crate's dev-dependencies unify it on), and that binary is
//! armed but inert: nothing stops unless [`POINT_VAR`] is set in its environment.
//!
//! With the feature, the first call reads [`POINT_VAR`] once. Its grammar is
//! `<point>[@<phase>][#<attempt>]`, where `<point>` is a [`KillPoint::name`], and the optional
//! phase and attempt narrow the match to one step of a walk (for example `documented@prd#1`). A
//! matching call writes the marker file [`MARK_VAR`] names, written in full, synced and renamed
//! into place, so a reader never sees half of it. It then parks the calling thread until the
//! test's `SIGKILL`. A park still waiting after five minutes exits with [`PARK_EXPIRED_EXIT`]. A
//! malformed spec, or a spec with no marker path, exits with [`MISCONFIGURED_EXIT`] at the first
//! call, and so does a matching call whose marker cannot be written. That half is a private module, `armed`, named in plain text: a link to it would be a
//! `broken_intra_doc_links` error in any build without the feature.

use htui_core::model::RunStep;

/// The environment variable naming the point, and optionally the phase and attempt, to stop at.
pub const POINT_VAR: &str = "HTUI_TEST_KILL_POINT";
/// The environment variable naming the marker file a reached point writes.
pub const MARK_VAR: &str = "HTUI_TEST_KILL_MARK";
/// The exit code of a process whose parked kill point was never killed.
pub const PARK_EXPIRED_EXIT: i32 = 86;
/// The exit code of a process whose kill-point spec does not parse or names no marker, or whose
/// marker could not be written.
pub const MISCONFIGURED_EXIT: i32 = 87;

/// A place a crash test may stop the process at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillPoint {
    /// After the step's output document, before verify and capture (K3).
    Documented,
    /// After the step's `after` commits, before `output_of` and `finish_step` (K4).
    Captured,
    /// After the command poll picked a pending row, before its cancel is spawned (K5).
    CommandPicked,
}

impl KillPoint {
    /// The name [`POINT_VAR`] spells it with: `documented`, `captured`, `command_picked`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Documented => "documented",
            Self::Captured => "captured",
            Self::CommandPicked => "command_picked",
        }
    }
}

/// The step a point is reached for; both `None` outside a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site<'a> {
    /// `run_step.phase_name`.
    pub phase: Option<&'a str>,
    /// `run_step.attempt`.
    pub attempt: Option<i32>,
}

impl Site<'static> {
    /// No step: the command poll's site.
    pub const NONE: Self = Self {
        phase: None,
        attempt: None,
    };
}

impl<'a> Site<'a> {
    /// `step`'s phase and attempt.
    #[must_use]
    pub fn step(step: &'a RunStep) -> Self {
        Self {
            phase: Some(&step.phase_name),
            attempt: Some(step.attempt),
        }
    }
}

/// MOD-24 D1: a no-op unless `test-support` is on and [`POINT_VAR`] names this point and site.
#[inline]
pub fn reached(point: KillPoint, at: Site<'_>) {
    #[cfg(feature = "test-support")]
    armed::reached(point, at);
    #[cfg(not(feature = "test-support"))]
    let _ = (point, at);
}

/// The armed half: the spec's parser, the marker writer and the park (blueprint §1.2).
#[cfg(feature = "test-support")]
mod armed {
    use std::fs::{self, File};
    use std::io::{self, Write as _};
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    use super::{KillPoint, MARK_VAR, MISCONFIGURED_EXIT, PARK_EXPIRED_EXIT, POINT_VAR, Site};

    /// How long a reached point waits for its `SIGKILL` before the process ends itself.
    const PARK_LIMIT: Duration = Duration::from_secs(300);

    /// Every point, for the parser's name lookup.
    const POINTS: [KillPoint; 3] = [
        KillPoint::Documented,
        KillPoint::Captured,
        KillPoint::CommandPicked,
    ];

    /// A parsed [`POINT_VAR`]: the point, and the phase and attempt it is narrowed to.
    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct Spec {
        pub(super) point: KillPoint,
        pub(super) phase: Option<String>,
        pub(super) attempt: Option<i32>,
    }

    /// `<point>[@<phase>][#<attempt>]`, trimmed; the attempt is `>= 1`, the phase is not empty.
    pub(super) fn parse(text: &str) -> Result<Spec, String> {
        let text = text.trim();
        let refuse = |why: &str| Err(format!("{why}: `{text}`"));
        let (rest, attempt) = match text.rsplit_once('#') {
            None => (text, None),
            Some((rest, attempt)) => match attempt.parse::<i32>() {
                Ok(attempt) if attempt >= 1 => (rest, Some(attempt)),
                _ => return refuse("the attempt is not a whole number of at least 1"),
            },
        };
        let (name, phase) = match rest.split_once('@') {
            None => (rest, None),
            Some((_, "")) => return refuse("the phase after `@` is empty"),
            Some((name, phase)) => (name, Some(phase.to_owned())),
        };
        let Some(point) = POINTS.into_iter().find(|point| point.name() == name) else {
            return refuse("no kill point has this name");
        };
        Ok(Spec {
            point,
            phase,
            attempt,
        })
    }

    impl Spec {
        /// Whether a call at `point` and `at` is the one this spec stops at.
        pub(super) fn matches(&self, point: KillPoint, at: Site<'_>) -> bool {
            self.point == point
                && self
                    .phase
                    .as_deref()
                    .is_none_or(|phase| at.phase == Some(phase))
                && self
                    .attempt
                    .is_none_or(|attempt| at.attempt == Some(attempt))
        }
    }

    /// Writes `point`'s name to `path` whole: a sibling temp file, synced, then renamed.
    pub(super) fn mark(path: &Path, point: KillPoint) -> io::Result<()> {
        let temp = path.with_extension("tmp");
        let mut file = File::create(&temp)?;
        file.write_all(point.name().as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    }

    /// The spec and the marker path read from the environment, once.
    #[derive(Debug)]
    struct Armed {
        spec: Spec,
        mark: PathBuf,
    }

    static ARMED: OnceLock<Option<Armed>> = OnceLock::new();

    /// `None` when [`POINT_VAR`] is unset; a bad spec or a missing marker path ends the process.
    fn from_env() -> Option<Armed> {
        let text = std::env::var(POINT_VAR).ok()?;
        let spec = parse(&text).unwrap_or_else(|why| misconfigured(&why));
        let Some(mark) = std::env::var_os(MARK_VAR) else {
            misconfigured(&format!("{POINT_VAR} is set but {MARK_VAR} is not"));
        };
        Some(Armed {
            spec,
            mark: PathBuf::from(mark),
        })
    }

    /// Reports `why` and exits with [`MISCONFIGURED_EXIT`].
    fn misconfigured(why: &str) -> ! {
        eprintln!("htui kill point: {why}");
        std::process::exit(MISCONFIGURED_EXIT)
    }

    /// Marks and parks when the environment's spec names `point` at `at`; returns otherwise.
    pub(super) fn reached(point: KillPoint, at: Site<'_>) {
        let Some(armed) = ARMED.get_or_init(from_env) else {
            return;
        };
        if !armed.spec.matches(point, at) {
            return;
        }
        if let Err(err) = mark(&armed.mark, point) {
            misconfigured(&format!(
                "writing the marker `{}` failed: {err}",
                armed.mark.display()
            ));
        }
        let deadline = Instant::now() + PARK_LIMIT;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            std::thread::park_timeout(left);
        }
        std::process::exit(PARK_EXPIRED_EXIT)
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use super::armed::{Spec, mark, parse};
    use super::{KillPoint, Site};

    fn spec(point: KillPoint, phase: Option<&str>, attempt: Option<i32>) -> Spec {
        Spec {
            point,
            phase: phase.map(str::to_owned),
            attempt,
        }
    }

    fn at(phase: &str, attempt: i32) -> Site<'_> {
        Site {
            phase: Some(phase),
            attempt: Some(attempt),
        }
    }

    #[test]
    fn a_spec_names_a_point_and_optionally_a_phase_and_an_attempt() {
        assert_eq!(
            parse("documented"),
            Ok(spec(KillPoint::Documented, None, None))
        );
        assert_eq!(
            parse("captured@prd"),
            Ok(spec(KillPoint::Captured, Some("prd"), None))
        );
        assert_eq!(
            parse("command_picked"),
            Ok(spec(KillPoint::CommandPicked, None, None))
        );
        assert_eq!(
            parse("documented@prd#1"),
            Ok(spec(KillPoint::Documented, Some("prd"), Some(1)))
        );
        assert_eq!(
            parse("captured#2"),
            Ok(spec(KillPoint::Captured, None, Some(2)))
        );
        assert_eq!(
            parse("  documented@prd#1\n"),
            Ok(spec(KillPoint::Documented, Some("prd"), Some(1))),
            "the spec is trimmed"
        );
    }

    #[test]
    fn a_malformed_spec_is_refused() {
        for text in [
            "",
            "nap",
            "documented@",
            "documented@prd#0",
            "documented@prd#x",
            "captured@#1",
        ] {
            assert!(parse(text).is_err(), "`{text}` must not parse");
        }
    }

    #[test]
    fn a_spec_fires_only_for_its_point_phase_and_attempt() {
        let spec = parse("documented@prd#1").expect("parses");
        assert!(spec.matches(KillPoint::Documented, at("prd", 1)));
        assert!(!spec.matches(KillPoint::Captured, at("prd", 1)));
        assert!(!spec.matches(KillPoint::Documented, at("plan", 1)));
        assert!(!spec.matches(KillPoint::Documented, at("prd", 2)));
        assert!(!spec.matches(KillPoint::Documented, Site::NONE));
    }

    #[test]
    fn a_bare_point_matches_every_site_of_that_point() {
        let spec = parse("command_picked").expect("parses");
        assert!(spec.matches(KillPoint::CommandPicked, Site::NONE));
        assert!(spec.matches(KillPoint::CommandPicked, at("prd", 1)));
        assert!(spec.matches(KillPoint::CommandPicked, at("plan", 3)));
        assert!(!spec.matches(KillPoint::Documented, Site::NONE));
    }

    #[test]
    fn a_reached_point_leaves_the_whole_marker_and_no_temp_file() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("m");
        mark(&path, KillPoint::Captured).expect("the marker is written");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the marker exists"),
            "captured"
        );
        assert!(
            !path.with_extension("tmp").exists(),
            "the temp file is renamed away"
        );
    }

    #[test]
    fn the_names_round_trip() {
        for point in [
            KillPoint::Documented,
            KillPoint::Captured,
            KillPoint::CommandPicked,
        ] {
            assert_eq!(parse(point.name()), Ok(spec(point, None, None)));
        }
    }
}

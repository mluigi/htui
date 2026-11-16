//! MOD-132 M1 T1: the target channel's stand-in (`htui_agent::target_channel`).
//!
//! Every case runs a real process on this box under a throwaway `home`. Cases that need a POSIX
//! shell are `#[cfg(unix)]`; the rest drive `git`, which every supported box has.
#![cfg(feature = "test-support")]

use std::path::Path;
use std::time::{Duration, Instant};

use htui_agent::target_channel::{
    Canned, ChannelError, ExecOutput, ExecSpec, STAND_IN_MARKER, StandInChannel, StdinFrom,
    StdoutTo, TargetChannel,
};

const SHORT: Duration = Duration::from_secs(60);

fn spec(argv: &[&str]) -> ExecSpec {
    ExecSpec {
        verb: "probe",
        argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
        stdin: StdinFrom::Null,
        stdout: StdoutTo::Tail,
        timeout: SHORT,
    }
}

async fn run(channel: &StandInChannel, spec: &ExecSpec) -> Result<ExecOutput, ChannelError> {
    channel.exec(spec).await
}

#[tokio::test]
async fn describe_names_the_stand_in_and_its_home() {
    let home = tempfile::tempdir().expect("home");
    let channel = StandInChannel::new(home.path());
    assert_eq!(channel.home(), home.path());
    let said = channel.describe();
    assert!(said.starts_with("stand-in at "), "got {said:?}");
    assert!(
        said.contains(&home.path().display().to_string()),
        "got {said:?}"
    );
}

/// The inherited half of the environment pin: a test cannot plant a variable in its own process
/// (`set_var` is `unsafe`), so it re-runs itself, alone, as a child test binary that inherits
/// `HTUI_QDRANT_API_KEY`, `GIT_DIR` and the variables that hand `git` configuration from outside
/// the stand-in home, and the child execs through the stand-in and asserts.
#[cfg(unix)]
#[tokio::test]
async fn argv_runs_under_home_with_home_and_the_marker_set_and_htui_own_env_absent() {
    const CHILD: &str = "HTUI_AGENT_TARGET_CHANNEL_ENV_CHILD";
    const NAME: &str = "argv_runs_under_home_with_home_and_the_marker_set_and_htui_own_env_absent";
    const KEY_SENTINEL: &str = "target-channel-key-sentinel";
    let name = format!(
        "{}::{NAME}",
        module_path!().split_once("::").map_or("", |(_, rest)| rest)
    );
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([name.as_str(), "--exact", "--test-threads=1"])
            .env(CHILD, "1")
            .env(htui_core::secret::QDRANT_KEY_ENV, KEY_SENTINEL)
            .env("GIT_DIR", "/nonexistent-git-dir")
            .env("GIT_WORK_TREE", "/nonexistent-work-tree")
            .env("GIT_CONFIG_GLOBAL", "/nonexistent-global-config")
            .env("GIT_CONFIG_SYSTEM", "/nonexistent-system-config")
            .env("GIT_CONFIG_NOSYSTEM", "0")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "htui.leak")
            .env("GIT_CONFIG_VALUE_0", "count")
            .env("GIT_CONFIG_KEY_7", "htui.stray")
            .env("GIT_CONFIG_VALUE_7", "stray")
            .env("GIT_CONFIG_PARAMETERS", "'htui.leak'='parameters'")
            .env("XDG_CONFIG_HOME", "/nonexistent-xdg-config")
            .output()
            .expect("re-run the test binary");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "child failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // A filter that matched nothing would also succeed.
        assert!(stdout.contains("1 passed"), "child ran no test:\n{stdout}");
        return;
    }
    // The child: both are inherited, so without the strip the target command would see them.
    assert_eq!(
        std::env::var(htui_core::secret::QDRANT_KEY_ENV).as_deref(),
        Ok(KEY_SENTINEL)
    );
    let home = tempfile::tempdir().expect("home");
    let home_path = home.path().canonicalize().expect("canonical home");
    let channel = StandInChannel::new(&home_path);
    let script = format!(
        "pwd; echo \"home=$HOME\"; echo \"marker=${{{STAND_IN_MARKER}-unset}}\"; \
         echo \"key=${{{key}-unset}}\"; echo \"gitdir=${{GIT_DIR-unset}}\"; \
         echo \"worktree=${{GIT_WORK_TREE-unset}}\"; \
         echo \"global=${{GIT_CONFIG_GLOBAL-unset}}\"; echo \"system=${{GIT_CONFIG_SYSTEM-unset}}\"; \
         echo \"nosystem=${{GIT_CONFIG_NOSYSTEM-unset}}\"; echo \"count=${{GIT_CONFIG_COUNT-unset}}\"; \
         echo \"key0=${{GIT_CONFIG_KEY_0-unset}}\"; echo \"value0=${{GIT_CONFIG_VALUE_0-unset}}\"; \
         echo \"key7=${{GIT_CONFIG_KEY_7-unset}}\"; echo \"value7=${{GIT_CONFIG_VALUE_7-unset}}\"; \
         echo \"parameters=${{GIT_CONFIG_PARAMETERS-unset}}\"; echo \"xdg=${{XDG_CONFIG_HOME-unset}}\"; \
         echo \"leak=$(git config --get htui.leak || echo none)\"; mkdir rel",
        key = htui_core::secret::QDRANT_KEY_ENV,
    );
    let out = run(&channel, &spec(&["sh", "-c", &script]))
        .await
        .expect("exec");
    assert!(out.ok(), "{out:?}");
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        lines,
        [
            home_path.display().to_string().as_str(),
            format!("home={}", home_path.display()).as_str(),
            "marker=1",
            "key=unset",
            "gitdir=unset",
            "worktree=unset",
            "global=unset",
            "system=unset",
            "nosystem=1",
            "count=unset",
            "key0=unset",
            "value0=unset",
            "key7=unset",
            "value7=unset",
            "parameters=unset",
            format!("xdg={}/.config", home_path.display()).as_str(),
            "leak=none",
        ],
        "{out:?}"
    );
    assert!(
        home_path.join("rel").is_dir(),
        "a relative path lands under home"
    );
}

#[tokio::test]
async fn exit_codes_pass_through() {
    let home = tempfile::tempdir().expect("home");
    let channel = StandInChannel::new(home.path());

    let out = run(&channel, &spec(&["git", "--version"]))
        .await
        .expect("exec");
    assert_eq!(out.code, Some(0));
    assert!(out.ok());
    assert!(out.stdout.starts_with("git version"), "{out:?}");
    assert_eq!(out.stdout_bytes, out.stdout.len() as u64);

    // `home` is no repository: `rev-parse --verify` refuses with 128 and says why on stderr.
    let out = run(
        &channel,
        &spec(&["git", "-C", ".", "rev-parse", "--verify", "HEAD"]),
    )
    .await
    .expect("exec");
    assert_eq!(out.code, Some(128), "{out:?}");
    assert!(!out.ok());
    assert!(out.stderr.contains("not a git repository"), "{out:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn an_arbitrary_exit_code_passes_through() {
    let home = tempfile::tempdir().expect("home");
    let channel = StandInChannel::new(home.path());
    let out = run(&channel, &spec(&["sh", "-c", "echo said >&2; exit 7"]))
        .await
        .expect("exec");
    assert_eq!(out.code, Some(7));
    assert_eq!(out.stderr, "said\n");
    assert!(out.stdout.is_empty());
}

#[tokio::test]
async fn a_missing_program_is_a_spawn_error() {
    let home = tempfile::tempdir().expect("home");
    let channel = StandInChannel::new(home.path());
    let err = run(&channel, &spec(&["htui-no-such-program-anywhere"]))
        .await
        .expect_err("no such program");
    assert!(matches!(err, ChannelError::Spawn(_)), "{err:?}");
    let err = run(&channel, &spec(&[])).await.expect_err("empty argv");
    assert!(matches!(err, ChannelError::Spawn(_)), "{err:?}");
}

#[tokio::test]
async fn stdin_comes_from_a_file_at_an_offset() {
    let home = tempfile::tempdir().expect("home");
    let scratch = tempfile::tempdir().expect("scratch");
    let input = scratch.path().join("input");
    std::fs::write(&input, b"HEADER-TO-SKIP\npayload bytes\n").expect("write input");
    let channel = StandInChannel::new(home.path());

    let mut hashing = spec(&["git", "hash-object", "--stdin"]);
    hashing.stdin = StdinFrom::File {
        path: input.clone(),
        offset: "HEADER-TO-SKIP\n".len() as u64,
    };
    let out = run(&channel, &hashing).await.expect("exec");
    assert!(out.ok(), "{out:?}");

    let expected = std::process::Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(
            std::fs::File::open(write(scratch.path(), "tail", b"payload bytes\n")).expect("open"),
        )
        .output()
        .expect("git hash-object");
    assert_eq!(out.stdout, String::from_utf8_lossy(&expected.stdout));

    // A missing stdin file is an I/O error before anything runs.
    hashing.stdin = StdinFrom::File {
        path: scratch.path().join("absent"),
        offset: 0,
    };
    let err = run(&channel, &hashing).await.expect_err("absent stdin");
    assert!(matches!(err, ChannelError::Io(_)), "{err:?}");
}

/// More than a pipe buffer holds goes through whole, from the offset.
#[cfg(unix)]
#[tokio::test]
async fn a_large_stdin_streams_whole_from_its_offset() {
    let home = tempfile::tempdir().expect("home");
    let scratch = tempfile::tempdir().expect("scratch");
    let mut bytes = vec![b'h'; 100];
    bytes.extend(std::iter::repeat_n(b'p', 3 * 1024 * 1024));
    let input = write(scratch.path(), "big", &bytes);
    let channel = StandInChannel::new(home.path());
    let mut counting = spec(&["sh", "-c", "wc -c; echo done >&2"]);
    counting.stdin = StdinFrom::File {
        path: input,
        offset: 100,
    };
    let out = run(&channel, &counting).await.expect("exec");
    assert!(out.ok(), "{out:?}");
    assert_eq!(
        out.stdout.trim(),
        (3 * 1024 * 1024).to_string(),
        "every byte after the offset, and only those: {out:?}"
    );
    assert_eq!(out.stderr, "done\n");
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write");
    path
}

#[tokio::test]
async fn stdout_to_a_file_is_new_private_and_counted() {
    let home = tempfile::tempdir().expect("home");
    let scratch = tempfile::tempdir().expect("scratch");
    let target = scratch.path().join("out.bin");
    let channel = StandInChannel::new(home.path());

    let mut version = spec(&["git", "--version"]);
    version.stdout = StdoutTo::File {
        path: target.clone(),
        cap: 1024,
    };
    let out = run(&channel, &version).await.expect("exec");
    assert!(out.ok(), "{out:?}");
    assert!(out.stdout.is_empty(), "stdout went to the file: {out:?}");
    let written = std::fs::read(&target).expect("read the file");
    assert!(written.starts_with(b"git version"), "{written:?}");
    assert_eq!(out.stdout_bytes, written.len() as u64);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&target)
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
    }

    // `create_new`: an existing file is refused, left as it was, and nothing ran.
    let err = run(&channel, &version).await.expect_err("file exists");
    match err {
        ChannelError::Io(io) => assert_eq!(io.kind(), std::io::ErrorKind::AlreadyExists),
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
    assert_eq!(std::fs::read(&target).expect("still there"), written);
}

#[cfg(unix)]
#[tokio::test]
async fn over_the_cap_the_child_is_killed_and_the_file_removed() {
    let home = tempfile::tempdir().expect("home");
    let scratch = tempfile::tempdir().expect("scratch");
    let target = scratch.path().join("out.bin");
    let pid_file = scratch.path().join("pid");
    let channel = StandInChannel::new(home.path());

    // `yes` never stops on its own: only the cap ends it.
    let script = format!("echo $$ > '{}'; exec yes", pid_file.display());
    let mut endless = spec(&["sh", "-c", &script]);
    endless.verb = "endless";
    endless.stdout = StdoutTo::File {
        path: target.clone(),
        cap: 4096,
    };
    let started = Instant::now();
    let err = run(&channel, &endless).await.expect_err("over the cap");
    assert!(
        matches!(
            err,
            ChannelError::OverCap {
                verb: "endless",
                cap: 4096
            }
        ),
        "{err:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the cap, not the deadline"
    );
    assert!(!target.exists(), "the over-cap file is removed");
    assert_gone(&pid_file).await;

    // Exactly the cap is not over it.
    let mut exact = spec(&["sh", "-c", "printf 0123456789"]);
    exact.stdout = StdoutTo::File {
        path: target.clone(),
        cap: 10,
    };
    let out = run(&channel, &exact).await.expect("exactly the cap");
    assert_eq!(out.stdout_bytes, 10);
    assert_eq!(std::fs::read(&target).expect("read"), b"0123456789");
}

#[cfg(unix)]
#[tokio::test]
async fn the_deadline_kills_a_sleep_and_its_child() {
    let home = tempfile::tempdir().expect("home");
    let scratch = tempfile::tempdir().expect("scratch");
    let pid_file = scratch.path().join("pid");
    let channel = StandInChannel::new(home.path());

    // The shell's background `sleep` is its child: the group kill must reach it too.
    let script = format!(
        "sleep 300 & echo $! > '{pid}.tmp'; mv '{pid}.tmp' '{pid}'; sleep 300",
        pid = pid_file.display()
    );
    let mut sleeper = spec(&["sh", "-c", &script]);
    sleeper.verb = "sleeper";
    sleeper.timeout = Duration::from_secs(2);
    let started = Instant::now();
    let err = run(&channel, &sleeper).await.expect_err("timed out");
    assert!(
        matches!(err, ChannelError::TimedOut { verb: "sleeper", after } if after == Duration::from_secs(2)),
        "{err:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "{:?}",
        started.elapsed()
    );
    assert_gone(&pid_file).await;
}

/// The process whose pid `pid_file` holds is dead (gone, or a zombie nobody reaped yet) within
/// a few seconds.
#[cfg(unix)]
async fn assert_gone(pid_file: &Path) {
    let pid = std::fs::read_to_string(pid_file).expect("the pid was written");
    let pid = pid.trim();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !alive(pid) {
            return;
        }
        assert!(Instant::now() < deadline, "pid {pid} outlived the kill");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(unix)]
fn alive(pid: &str) -> bool {
    if cfg!(target_os = "linux") {
        // A zombie still answers `kill -0`; its state letter says it is dead.
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => !stat
                .rsplit_once(')')
                .is_some_and(|(_, rest)| rest.trim_start().starts_with('Z')),
            Err(_) => false,
        }
    } else {
        std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

#[tokio::test]
async fn unreachable_fails_every_exec() {
    let home = tempfile::tempdir().expect("home");
    let channel = StandInChannel::new(home.path()).unreachable();
    let marker = home.path().join("ran");
    let err = run(
        &channel,
        &spec(&["git", "init", "--quiet", &marker.display().to_string()]),
    )
    .await
    .expect_err("unreachable");
    match err {
        ChannelError::Unreachable(said) => {
            assert!(said.contains(&channel.describe()), "{said:?}");
        }
        other => panic!("expected Unreachable, got {other:?}"),
    }
    assert!(!marker.exists(), "nothing ran");
}

#[tokio::test]
async fn a_canned_answer_replaces_the_run() {
    let home = tempfile::tempdir().expect("home");
    let scratch = tempfile::tempdir().expect("scratch");
    let marker = home.path().join("ran");
    let channel = StandInChannel::new(home.path()).canned(|argv| {
        (argv.get(1).map(String::as_str) == Some("init")).then(|| Canned {
            stdout: b"crafted bytes\n".to_vec(),
            code: 3,
        })
    });

    let out = run(
        &channel,
        &spec(&["git", "init", "--quiet", &marker.display().to_string()]),
    )
    .await
    .expect("canned");
    assert_eq!(out.code, Some(3));
    assert_eq!(out.stdout, "crafted bytes\n");
    assert_eq!(out.stdout_bytes, 14);
    assert!(out.stderr.is_empty());
    assert!(!marker.exists(), "the canned argv did not run");

    // An argv the matcher declines still runs.
    let out = run(&channel, &spec(&["git", "--version"]))
        .await
        .expect("real");
    assert!(out.stdout.starts_with("git version"), "{out:?}");

    // Through a file, with the cap: within it the bytes land, past it the file is removed.
    let target = scratch.path().join("canned.bin");
    let mut to_file = spec(&["git", "init"]);
    to_file.stdout = StdoutTo::File {
        path: target.clone(),
        cap: 14,
    };
    let out = run(&channel, &to_file).await.expect("within the cap");
    assert_eq!(out.stdout_bytes, 14);
    assert!(out.stdout.is_empty());
    assert_eq!(std::fs::read(&target).expect("read"), b"crafted bytes\n");

    let over = scratch.path().join("over.bin");
    to_file.stdout = StdoutTo::File {
        path: over.clone(),
        cap: 13,
    };
    to_file.verb = "canned";
    let err = run(&channel, &to_file).await.expect_err("over the cap");
    assert!(
        matches!(
            err,
            ChannelError::OverCap {
                verb: "canned",
                cap: 13
            }
        ),
        "{err:?}"
    );
    assert!(!over.exists(), "the over-cap file is removed");
}

#[test]
fn the_channel_is_dyn_compatible() {
    let home = tempfile::tempdir().expect("home");
    let boxed: Box<dyn TargetChannel> = Box::new(StandInChannel::new(home.path()));
    assert!(boxed.describe().starts_with("stand-in at "));
}

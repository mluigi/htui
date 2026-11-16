//! MOD-132 M1 T4: [`collect`] brings a target clone's run branch back to this box as a bundle
//! (plan P6, P8, P9), over the test-only [`StandInChannel`], from clones made by T3's [`seed`].
//!
//! Every case opens with [`skip_without_git`] and passes on a box without one, and every case
//! ends on [`World::assert_clean`]: no `refs/htui-collect/*` ref and nothing in the scratch (no
//! bundle, no object quarantine) remain.

use std::path::{Path, PathBuf};
use std::process::Output;

use htui_agent::target_channel::{Canned, ChannelError, STAND_IN_MARKER, StandInChannel};
use htui_orch::isolate::IsolateError;
use htui_orch::isolate::git::testkit::repo_with_one_commit;
use htui_orch::isolate::git::{SCRUBBED_ENV, branch_target, create_branch, create_ref, has_commit};
use htui_orch::skip_without_git;
use htui_orch::target::bundle::BundleRefusal;
use htui_orch::target::collect::{
    COLLECT_CAP, CollectOutcome, CollectRefusal, CollectRequest, Consent, HOST_EXECUTING_PATHS,
    TEMP_REF_PREFIX, collect, collect_racing, display_path, host_executing, not_a_commit,
    sweep_temp_refs,
};
use htui_orch::target::seed::{SeedRequest, seed, target_git_failed};

use crate::write_executable::write_executable;

const RUN_BRANCH: &str = "htui/run/one";
const CLONE: &str = "clone";

/// One test's directories: this box's checkout, the scratch the bundle goes to, the stand-in's
/// home holding the seeded clone, and an empty home the oracle `git` runs under.
struct World {
    root: tempfile::TempDir,
    host: PathBuf,
    scratch: PathBuf,
    home: PathBuf,
    oracle_home: PathBuf,
    base: String,
}

async fn world() -> Option<(htui_orch::isolate::git::Cli, World)> {
    let cli = skip_without_git!()?;
    let root = tempfile::tempdir().expect("a temporary directory");
    let host = root.path().join("host");
    let scratch = root.path().join("scratch");
    let home = root.path().join("target-home");
    let oracle_home = root.path().join("oracle-home");
    for dir in [&scratch, &home, &oracle_home] {
        std::fs::create_dir_all(dir).expect("a directory");
    }
    let base = repo_with_one_commit(&host);
    let world = World {
        root,
        host,
        scratch,
        home,
        oracle_home,
        base,
    };
    let base_ref = world.stdout(&world.host, &["symbolic-ref", "HEAD"]);
    let seeded = seed(
        &cli,
        &StandInChannel::new(&world.home),
        &SeedRequest {
            repo: world.host.clone(),
            base_ref,
            target_dir: CLONE.to_owned(),
            run_branch: RUN_BRANCH.to_owned(),
            remote_url: None,
            scratch: world.scratch.clone(),
        },
    )
    .await
    .expect("the clone is seeded");
    assert_eq!(seeded.base, world.base);
    Some((cli, world))
}

impl World {
    fn clone_dir(&self) -> PathBuf {
        self.home.join(CLONE)
    }

    fn channel(&self) -> StandInChannel {
        StandInChannel::new(&self.home)
    }

    fn request(&self) -> CollectRequest {
        CollectRequest {
            repo: self.host.clone(),
            target_dir: CLONE.to_owned(),
            run_branch: RUN_BRANCH.to_owned(),
            base: self.base.clone(),
            local_branch: RUN_BRANCH.to_owned(),
            extra_confirm_paths: Vec::new(),
            scratch: self.scratch.clone(),
            cap: COLLECT_CAP,
            consent: None,
        }
    }

    /// The oracle: `git` in `dir`, with the location variables scrubbed, an empty home and
    /// [`STAND_IN_MARKER`] unset, so it reads `dir`'s own configuration and nothing of this
    /// box's.
    fn git(&self, dir: &Path, args: &[&str]) -> Output {
        self.git_with_input(dir, args, None)
    }

    /// [`World::git`], fed `input` on stdin when it is given.
    fn git_with_input(&self, dir: &Path, args: &[&str], input: Option<&[u8]>) -> Output {
        use std::io::Write as _;

        let mut command = std::process::Command::new("git");
        command.arg("-C").arg(dir).args(args);
        for key in SCRUBBED_ENV {
            command.env_remove(key);
        }
        command
            .env_remove(STAND_IN_MARKER)
            .env("HOME", &self.oracle_home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(if input.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = command.spawn().expect("git runs");
        if let Some(input) = input {
            let mut stdin = child.stdin.take().expect("a stdin pipe");
            stdin.write_all(input).expect("the input is written");
        }
        child.wait_with_output().expect("git exits")
    }

    /// The pack `git pack-objects --stdout` makes in `dir` of exactly `objects`.
    fn pack_of(&self, dir: &Path, objects: &[String]) -> Vec<u8> {
        let input = objects.join("\n") + "\n";
        let out = self.git_with_input(
            dir,
            &["pack-objects", "--stdout", "-q"],
            Some(input.as_bytes()),
        );
        assert!(out.status.success(), "pack-objects: {out:?}");
        out.stdout
    }

    /// Every object `git rev-list --objects <revs>` names in `dir`.
    fn objects_of(&self, dir: &Path, revs: &[&str]) -> Vec<String> {
        let mut args = vec!["rev-list", "--objects"];
        args.extend_from_slice(revs);
        self.stdout(dir, &args)
            .lines()
            .map(|line| line.split(' ').next().expect("a hash").to_owned())
            .collect()
    }

    /// A v2 bundle whose one prerequisite is the base and whose one ref is the run branch at
    /// `tip`, carrying `pack`.
    fn crafted_bundle(&self, tip: &str, pack: &[u8]) -> Vec<u8> {
        let mut bytes = format!(
            "# v2 git bundle\n-{}\n{tip} refs/heads/{RUN_BRANCH}\n\n",
            self.base
        )
        .into_bytes();
        bytes.extend_from_slice(pack);
        bytes
    }

    /// How many packs this box's checkout holds.
    fn packs(&self) -> usize {
        let dir = self.host.join(".git/objects/pack");
        std::fs::read_dir(&dir).map_or(0, |entries| {
            entries
                .filter(|entry| {
                    entry
                        .as_ref()
                        .expect("an entry")
                        .path()
                        .extension()
                        .is_some_and(|ext| ext == "pack")
                })
                .count()
        })
    }

    fn stdout(&self, dir: &Path, args: &[&str]) -> String {
        let out = self.git(dir, args);
        assert!(out.status.success(), "git {args:?} in {dir:?}: {out:?}");
        String::from_utf8(out.stdout)
            .expect("utf-8")
            .trim()
            .to_owned()
    }

    /// Commits `files` (`(path, body)`, nested paths allowed) on the clone's run branch and
    /// returns the new tip.
    fn commit_in_clone(&self, files: &[(&str, &str)], message: &str) -> String {
        let clone = self.clone_dir();
        for (path, body) in files {
            let file = clone.join(path);
            std::fs::create_dir_all(file.parent().expect("a parent")).expect("a directory");
            std::fs::write(&file, body).expect("the file is written");
            self.stdout(&clone, &["add", "--", path]);
        }
        self.stdout(
            &clone,
            &[
                "-c",
                "user.name=Agent",
                "-c",
                "user.email=agent@localhost",
                "commit",
                "--quiet",
                "--no-verify",
                "-m",
                message,
            ],
        );
        self.stdout(&clone, &["rev-parse", "HEAD"])
    }

    /// The clone's run branch, reset to `to` (a rewrite when it drops collected commits).
    fn reset_clone(&self, to: &str) {
        self.stdout(&self.clone_dir(), &["reset", "--quiet", "--hard", to]);
    }

    fn local(&self) -> Option<String> {
        branch_target(&self.host, RUN_BRANCH).expect("the branch reads")
    }

    /// The bytes `git bundle create - refs/heads/<run> ^<base>` writes in the clone.
    fn real_bundle(&self) -> Vec<u8> {
        let out = self.git(
            &self.clone_dir(),
            &[
                "bundle",
                "create",
                "-",
                &format!("refs/heads/{RUN_BRANCH}"),
                &format!("^{}", self.base),
            ],
        );
        assert!(out.status.success(), "bundle create: {out:?}");
        out.stdout
    }

    /// No collect temp ref in this box's checkout, and an empty scratch.
    fn assert_clean(&self) {
        assert_eq!(
            self.stdout(&self.host, &["for-each-ref", TEMP_REF_PREFIX]),
            "",
            "a collect temp ref was left behind"
        );
        let left: Vec<_> = std::fs::read_dir(&self.scratch)
            .expect("the scratch reads")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        assert!(left.is_empty(), "the scratch still holds {left:?}");
    }
}

/// A stand-in whose `rev-parse` answers `tip` and whose `bundle create` answers `bytes`.
fn canned_tip_and_bundle(world: &World, tip: &str, bytes: Vec<u8>) -> StandInChannel {
    let answer = format!("{tip}\n").into_bytes();
    canned_bundle(world, bytes).canned(move |argv: &[String]| {
        argv.iter().any(|arg| arg == "rev-parse").then(|| Canned {
            stdout: answer.clone(),
            code: 0,
        })
    })
}

/// A stand-in whose `bundle create` answers `bytes` instead of running.
fn canned_bundle(world: &World, bytes: Vec<u8>) -> StandInChannel {
    world.channel().canned(move |argv: &[String]| {
        argv.windows(2)
            .any(|pair| pair[0] == "bundle" && pair[1] == "create")
            .then(|| Canned {
                stdout: bytes.clone(),
                code: 0,
            })
    })
}

fn consent(tip: &str, from: Option<&str>, force: bool, config_paths: &[&str]) -> Option<Consent> {
    Some(Consent {
        tip: tip.to_owned(),
        from: from.map(str::to_owned),
        force,
        config_paths: config_paths.iter().map(|path| (*path).to_owned()).collect(),
    })
}

/// R1b M2: earlier builds' temp refs are swept, and nothing else is.
#[test]
fn sweep_temp_refs_removes_every_leftover_and_nothing_else() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let repo = root.path().join("repo");
    let base = repo_with_one_commit(&repo);
    for name in [
        "refs/htui-collect/00000000-0000-0000-0000-000000000001/00000000-0000-0000-0000-000000000002",
        "refs/htui-collect/00000000-0000-0000-0000-000000000003/00000000-0000-0000-0000-000000000004",
        "refs/heads/htui-collect/kept",
    ] {
        create_ref(&repo, name, &base).expect("a ref");
    }
    assert_eq!(sweep_temp_refs(&repo).expect("the sweep"), 2);
    assert_eq!(sweep_temp_refs(&repo).expect("the second sweep"), 0);
    assert_eq!(
        branch_target(&repo, "htui-collect/kept").expect("the branch reads"),
        Some(base)
    );
    assert!(TEMP_REF_PREFIX.starts_with("refs/"));
    assert_eq!(COLLECT_CAP, 2 * 1024 * 1024 * 1024);
}

/// R1b L2: target-supplied text reaches a person escaped.
#[test]
fn control_characters_are_escaped_for_a_person() {
    assert_eq!(
        display_path(".claude/\u{1b}[2Jx\u{7f}\u{85}\u{2028}\u{2029}\ty\nz"),
        ".claude/\\u{1b}[2Jx\\u{7f}\\u{85}\\u{2028}\\u{2029}\\u{9}y\\u{a}z"
    );
    assert_eq!(display_path("plain/ünïcode.md"), "plain/ünïcode.md");
    let sentence = target_git_failed("bundle create", "clone", "fatal: \u{1b}]0;owned\u{7}");
    assert!(
        sentence.ends_with("fatal: \\u{1b}]0;owned\\u{7}"),
        "{sentence}"
    );
    assert!(!sentence.chars().any(char::is_control), "{sentence}");
}

#[test]
fn host_executing_paths_match_prefixes_root_files_and_basenames() {
    let changed: Vec<String> = [
        ".claude/settings.json",
        ".claude",
        "src/.claude/x",
        ".vscode/tasks.json",
        "Makefile",
        "sub/Makefile",
        "package.json",
        "web/package.json",
        "sub/dir/AGENTS.md",
        "CLAUDE.md",
        "crates/x/build.rs",
        "crates/x/src/lib.rs",
        "docs/.gitattributes",
        ".Husky/pre-commit",
        "ops/deploy.sh",
        "ops",
        "scripts/run.sh",
        "README.md",
        "CLAUDE.md",
    ]
    .iter()
    .map(|path| (*path).to_owned())
    .collect();
    let extra = vec!["ops/".to_owned(), "scripts/run.sh".to_owned()];
    assert_eq!(
        host_executing(&changed, &extra),
        vec![
            ".Husky/pre-commit",
            ".claude",
            ".claude/settings.json",
            ".vscode/tasks.json",
            "CLAUDE.md",
            "Makefile",
            "crates/x/build.rs",
            "docs/.gitattributes",
            "ops",
            "ops/deploy.sh",
            "package.json",
            "scripts/run.sh",
            "sub/dir/AGENTS.md",
            "web/package.json",
        ]
    );
    assert!(host_executing(&changed[..0], &extra).is_empty());
    assert!(HOST_EXECUTING_PATHS.prefixes.contains(&".claude/"));
    assert!(HOST_EXECUTING_PATHS.root_files.contains(&".mcp.json"));
    assert!(HOST_EXECUTING_PATHS.basenames.contains(&"build.rs"));
    assert!(
        HOST_EXECUTING_PATHS
            .prefixes
            .iter()
            .all(|prefix| prefix.ends_with('/'))
    );
}

/// R1b M6: `.cargo/` at any depth, and the tool files that run code in their own directory as
/// basenames anywhere; `Cargo.toml` is not listed. R1b L1: a name a case-folding filesystem, or
/// NTFS's trailing-dot rule, reads as a listed one is listed too.
#[test]
fn widened_rules_match_at_any_depth_and_beyond_ascii() {
    let changed: Vec<String> = [
        "crates/x/.cargo/config.toml",
        "crates/x/.cargo",
        "sub/rust-toolchain.toml",
        "sub/rust-toolchain",
        "packages/a/package.json",
        "sub/.envrc",
        "crates/x/Cargo.toml",
        "Cargo.lock",
        "crates/x/cargo/config.toml",
        ".hu\u{17f}ky/x",
        "CLAUDE.md.",
        "docs/AGENTS.md ",
        ".claude./settings.json",
        "pac\u{212a}age.json",
        "src/main.rs",
    ]
    .iter()
    .map(|path| (*path).to_owned())
    .collect();
    let extra = vec!["Ops/".to_owned(), "Scripts/Run.sh".to_owned()];
    let mut more = changed.clone();
    more.extend(["ops./deploy.sh", "scripts/run.sh.", "ops"].map(str::to_owned));
    let mut expected = vec![
        ".claude./settings.json",
        ".hu\u{17f}ky/x",
        "CLAUDE.md.",
        "crates/x/.cargo",
        "crates/x/.cargo/config.toml",
        "docs/AGENTS.md ",
        "pac\u{212a}age.json",
        "packages/a/package.json",
        "sub/.envrc",
        "sub/rust-toolchain",
        "sub/rust-toolchain.toml",
    ];
    expected.sort_unstable();
    assert_eq!(host_executing(&changed, &extra), expected);
    expected.extend(["ops", "ops./deploy.sh", "scripts/run.sh."]);
    expected.sort_unstable();
    assert_eq!(host_executing(&more, &extra), expected);
}

#[tokio::test]
async fn nothing_then_created_then_moved() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let channel = world.channel();

    let outcome = collect(&cli, &channel, &world.request()).await;
    assert_eq!(outcome.expect("collect"), CollectOutcome::Nothing);
    assert_eq!(world.local(), None, "nothing was created");
    world.assert_clean();

    let first = world.commit_in_clone(&[("a.txt", "a\n")], "a");
    let outcome = collect(&cli, &channel, &world.request()).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::Created { tip: first.clone() }
    );
    assert_eq!(world.local(), Some(first.clone()));
    assert!(has_commit(&world.host, &first).expect("lookup"));
    world.assert_clean();

    let second = world.commit_in_clone(&[("b.txt", "b\n")], "b");
    let outcome = collect(&cli, &channel, &world.request()).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::Moved {
            from: first,
            tip: second.clone(),
        }
    );
    assert_eq!(world.local(), Some(second));
    world.assert_clean();
}

#[tokio::test]
async fn a_rewrite_needs_a_forcing_consent_for_its_tip() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let channel = world.channel();
    let first = world.commit_in_clone(&[("a.txt", "a\n")], "a");
    collect(&cli, &channel, &world.request())
        .await
        .expect("the first collect");

    world.reset_clone(&world.base);
    let rewritten = world.commit_in_clone(&[("c.txt", "c\n")], "c");
    let needs = CollectOutcome::NeedsConsent {
        tip: rewritten.clone(),
        rewritten_from: Some(first.clone()),
        config_paths: Vec::new(),
    };

    let outcome = collect(&cli, &channel, &world.request()).await;
    assert_eq!(outcome.expect("collect"), needs);
    assert_eq!(world.local(), Some(first.clone()), "nothing moved");
    world.assert_clean();

    let mut req = world.request();
    req.consent = consent(&rewritten, Some(&first), false, &[]);
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(outcome.expect("collect"), needs, "consent without force");
    assert_eq!(world.local(), Some(first.clone()));
    world.assert_clean();

    req.consent = consent(&first, Some(&first), true, &[]);
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(outcome.expect("collect"), needs, "consent for another tip");
    world.assert_clean();

    req.consent = consent(&rewritten, None, true, &[]);
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(
        outcome.expect("collect"),
        needs,
        "consent over another local tip"
    );
    world.assert_clean();

    req.consent = consent(&rewritten, Some(&first), true, &[]);
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::Forced {
            from: first,
            tip: rewritten.clone(),
        }
    );
    assert_eq!(world.local(), Some(rewritten));
    world.assert_clean();
}

#[tokio::test]
async fn a_host_executing_path_needs_consent_for_its_tip_and_list() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let channel = world.channel();
    let first = world.commit_in_clone(
        &[(".claude/settings.json", "{}\n"), ("src/lib.rs", "\n")],
        "settings",
    );
    let packs = world.packs();

    let outcome = collect(&cli, &channel, &world.request()).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::NeedsConsent {
            tip: first.clone(),
            rewritten_from: None,
            config_paths: vec![".claude/settings.json".to_owned()],
        }
    );
    assert_eq!(world.local(), None, "nothing was created");
    assert_eq!(world.packs(), packs, "no pack was kept");
    assert!(!has_commit(&world.host, &first).expect("lookup"));
    world.assert_clean();

    let mut req = world.request();
    req.consent = consent(&world.base, None, false, &[".claude/settings.json"]);
    let outcome = collect(&cli, &channel, &req).await;
    assert!(
        matches!(outcome, Ok(CollectOutcome::NeedsConsent { .. })),
        "consent for another tip is ignored: {outcome:?}"
    );
    req.consent = consent(&first, None, false, &[]);
    let outcome = collect(&cli, &channel, &req).await;
    assert!(
        matches!(outcome, Ok(CollectOutcome::NeedsConsent { .. })),
        "consent for another list is ignored: {outcome:?}"
    );
    assert_eq!(world.local(), None);
    world.assert_clean();

    req.consent = consent(&first, None, false, &[".claude/settings.json"]);
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::Created { tip: first.clone() }
    );
    assert!(
        has_commit(&world.host, &first).expect("lookup"),
        "the tip's objects moved in"
    );
    assert_eq!(world.packs(), packs + 1);
    world.assert_clean();

    let second = world.commit_in_clone(
        &[
            ("sub/dir/AGENTS.md", "be nice\n"),
            ("build.rs", "fn main() {}\n"),
            ("ops/deploy.sh", "true\n"),
            ("README.md", "hi\n"),
        ],
        "more",
    );
    let mut req = world.request();
    req.extra_confirm_paths = vec!["ops/".to_owned()];
    let listed = vec![
        "build.rs".to_owned(),
        "ops/deploy.sh".to_owned(),
        "sub/dir/AGENTS.md".to_owned(),
    ];
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::NeedsConsent {
            tip: second.clone(),
            rewritten_from: None,
            config_paths: listed.clone(),
        }
    );
    assert_eq!(world.local(), Some(first.clone()));

    req.consent = Some(Consent {
        tip: second.clone(),
        from: Some(first.clone()),
        force: false,
        config_paths: listed,
    });
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::Moved {
            from: first,
            tip: second.clone(),
        }
    );
    assert_eq!(world.local(), Some(second));
    world.assert_clean();
}

/// P9: a changed name that is not UTF-8 is still matched (on its lossy decoding, which keeps
/// every ASCII byte the patterns are made of), so a skill or a `build.rs` under such a name cannot
/// slip past consent; a non-UTF-8 name outside the patterns is not listed.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_non_utf8_host_executing_path_still_needs_consent() {
    use std::os::unix::ffi::OsStrExt as _;

    let Some((cli, world)) = world().await else {
        return;
    };
    let channel = world.channel();
    let clone = world.clone_dir();
    for name in [
        &b".claude/skills/ev\xffil/SKILL.md"[..],
        b"cr\xfeate/build.rs",
        b"docs/n\xffote.txt",
    ] {
        let file = clone.join(std::ffi::OsStr::from_bytes(name));
        std::fs::create_dir_all(file.parent().expect("a parent")).expect("a directory");
        std::fs::write(&file, "x\n").expect("the file is written");
    }
    world.stdout(&clone, &["add", "--all"]);
    world.stdout(
        &clone,
        &[
            "-c",
            "user.name=Agent",
            "-c",
            "user.email=agent@localhost",
            "commit",
            "--quiet",
            "--no-verify",
            "-m",
            "non-utf-8 names",
        ],
    );
    let tip = world.stdout(&clone, &["rev-parse", "HEAD"]);

    let listed = vec![
        ".claude/skills/ev\u{fffd}il/SKILL.md".to_owned(),
        "cr\u{fffd}ate/build.rs".to_owned(),
    ];
    let outcome = collect(&cli, &channel, &world.request()).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::NeedsConsent {
            tip: tip.clone(),
            rewritten_from: None,
            config_paths: listed.clone(),
        }
    );
    assert_eq!(world.local(), None, "nothing was created");
    world.assert_clean();

    let mut req = world.request();
    req.consent = Some(Consent {
        tip: tip.clone(),
        from: None,
        force: false,
        config_paths: listed,
    });
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::Created { tip: tip.clone() }
    );
    assert_eq!(world.local(), Some(tip));
    world.assert_clean();
}

#[tokio::test]
async fn a_local_branch_checked_out_in_a_worktree_is_refused() {
    let Some((cli, world)) = world().await else {
        return;
    };
    world.commit_in_clone(&[("a.txt", "a\n")], "a");
    let tree = world.root.path().join("tree");
    world.stdout(
        &world.host,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            RUN_BRANCH,
            tree.to_str().expect("utf-8"),
        ],
    );

    let err = collect(&cli, &world.channel(), &world.request())
        .await
        .expect_err("a checked-out branch is refused");
    match &err {
        CollectRefusal::CheckedOut { branch, at } => {
            assert_eq!(branch, RUN_BRANCH);
            assert_eq!(
                at.canonicalize().expect("canonical"),
                tree.canonicalize().expect("canonical")
            );
        }
        other => panic!("expected CheckedOut, got {other:?}"),
    }
    assert!(err.to_string().contains(RUN_BRANCH), "{err}");
    assert_eq!(world.local(), Some(world.base.clone()), "nothing moved");
    world.assert_clean();
}

#[tokio::test]
async fn a_local_move_between_the_decision_and_the_swap_is_refused() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let channel = world.channel();
    world.commit_in_clone(&[("a.txt", "a\n")], "a");

    // The branch is absent when the decision reads it and created before the swap.
    let host = world.host.clone();
    let base = world.base.clone();
    let err = collect_racing(&cli, &channel, &world.request(), &move || {
        create_branch(&host, RUN_BRANCH, &base).expect("the racing create");
    })
    .await
    .expect_err("the lost race is refused");
    assert!(
        matches!(&err, CollectRefusal::LocalMoved { branch } if branch == RUN_BRANCH),
        "{err:?}"
    );
    assert_eq!(world.local(), Some(world.base.clone()));
    world.assert_clean();

    // The branch is at the base when the decision reads it and moved before the swap.
    let elsewhere = htui_orch::isolate::git::testkit::commit_file(&world.host, "h", "h\n", "h");
    let host = world.host.clone();
    let (from, to) = (world.base.clone(), elsewhere.clone());
    let err = collect_racing(&cli, &channel, &world.request(), &move || {
        htui_orch::isolate::git::move_branch(&host, RUN_BRANCH, &from, &to)
            .expect("the racing move");
    })
    .await
    .expect_err("the lost race is refused");
    assert!(matches!(&err, CollectRefusal::LocalMoved { .. }), "{err:?}");
    assert_eq!(world.local(), Some(elsewhere));
    world.assert_clean();
}

#[tokio::test]
async fn a_missing_run_branch_or_a_non_hash_answer_is_refused() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let mut req = world.request();
    req.run_branch = "htui/run/missing".to_owned();
    let err = collect(&cli, &world.channel(), &req)
        .await
        .expect_err("no such branch");
    assert!(
        matches!(&err, CollectRefusal::NoBranch { dir, branch }
            if dir == CLONE && branch == "htui/run/missing"),
        "{err:?}"
    );
    world.assert_clean();

    let channel = world.channel().canned(|argv: &[String]| {
        argv.iter().any(|arg| arg == "rev-parse").then(|| Canned {
            stdout: b"HEAD\n".to_vec(),
            code: 0,
        })
    });
    let err = collect(&cli, &channel, &world.request())
        .await
        .expect_err("not a hash");
    assert!(
        matches!(&err, CollectRefusal::NotAHash { got } if got == "HEAD"),
        "{err:?}"
    );
    world.assert_clean();
}

#[tokio::test]
async fn crafted_headers_are_refused_before_any_object_lands() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let tip = world.commit_in_clone(&[("a.txt", "a\n")], "a");
    let base = world.base.clone();
    let run = format!("refs/heads/{RUN_BRANCH}");
    let missing = "1234567890abcdef1234567890abcdef12345678";

    let header = |lines: &[String]| -> Vec<u8> {
        let mut bytes = lines.join("\n").into_bytes();
        bytes.extend_from_slice(b"\n\nPACK");
        bytes
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "an extra ref",
            header(&[
                "# v2 git bundle".to_owned(),
                format!("-{base}"),
                format!("{tip} {run}"),
                format!("{tip} refs/heads/other"),
            ]),
        ),
        (
            "the wrong ref",
            header(&[
                "# v2 git bundle".to_owned(),
                format!("-{base}"),
                format!("{tip} refs/heads/other"),
            ]),
        ),
        (
            "the tip changed",
            header(&[
                "# v2 git bundle".to_owned(),
                format!("-{base}"),
                format!("{base} {run}"),
            ]),
        ),
    ];
    for (what, bytes) in cases {
        let err = collect(&cli, &canned_bundle(&world, bytes), &world.request())
            .await
            .expect_err(what);
        match &err {
            CollectRefusal::Changed { expected, got } => {
                assert_eq!(expected, &format!("{tip} {run}"), "{what}");
                assert_ne!(expected, got, "{what}");
            }
            other => panic!("{what}: expected Changed, got {other:?}"),
        }
        world.assert_clean();
    }

    let bytes = header(&[
        "# v2 git bundle".to_owned(),
        format!("-{missing}"),
        format!("{tip} {run}"),
    ]);
    let err = collect(&cli, &canned_bundle(&world, bytes), &world.request())
        .await
        .expect_err("a missing prerequisite");
    assert!(
        matches!(&err, CollectRefusal::MissingPrerequisite { hash } if hash == missing),
        "{err:?}"
    );
    world.assert_clean();

    let bytes = header(&[
        "# v3 git bundle".to_owned(),
        "@filter=blob:none".to_owned(),
        format!("-{base}"),
        format!("{tip} {run}"),
    ]);
    let err = collect(&cli, &canned_bundle(&world, bytes), &world.request())
        .await
        .expect_err("a partial bundle");
    assert!(
        matches!(&err, CollectRefusal::Bundle(BundleRefusal::Filter(filter)) if filter == "blob:none"),
        "{err:?}"
    );
    world.assert_clean();

    assert_eq!(world.local(), None);
    assert!(!has_commit(&world.host, &tip).expect("lookup"));
}

#[tokio::test]
async fn a_truncated_pack_is_refused_and_keeps_nothing() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let tip = world.commit_in_clone(&[("a.txt", "a\n"), ("b.txt", "b\n")], "a");
    let mut bytes = world.real_bundle();
    let pack = bytes
        .windows(4)
        .position(|window| window == b"PACK")
        .expect("a pack");
    bytes.truncate(pack + (bytes.len() - pack) / 2);
    let packs = world.packs();

    let err = collect(&cli, &canned_bundle(&world, bytes), &world.request())
        .await
        .expect_err("a truncated pack");
    // `bundle verify` reads only the header and the prerequisites; the strict `index-pack`
    // finds the short pack.
    assert!(
        matches!(&err, CollectRefusal::FsckFailed(sentence) if sentence.contains("early EOF")),
        "{err:?}"
    );
    assert!(!has_commit(&world.host, &tip).expect("lookup"));
    assert_eq!(world.packs(), packs, "no pack was kept");
    assert_eq!(world.local(), None);
    world.assert_clean();
}

/// Plan B5/B6: a commit `git fsck` refuses (no author email) travels in a pack `git fetch` would
/// take; collect's strict `index-pack` refuses it and keeps nothing.
#[tokio::test]
async fn a_commit_fsck_refuses_is_refused_and_keeps_nothing() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let clone = world.clone_dir();
    let tree = world.stdout(&clone, &["rev-parse", &format!("{}^{{tree}}", world.base)]);
    let text = world.root.path().join("bad.txt");
    std::fs::write(
        &text,
        format!(
            "tree {tree}\nparent {}\nauthor Nobody 1600000000 +0000\n\
             committer Nobody <n@localhost> 1600000000 +0000\n\nno email\n",
            world.base
        ),
    )
    .expect("the commit text");
    let bad = world.stdout(
        &clone,
        &[
            "hash-object",
            "--literally",
            "-t",
            "commit",
            "-w",
            text.to_str().expect("utf-8"),
        ],
    );
    world.stdout(
        &clone,
        &["update-ref", &format!("refs/heads/{RUN_BRANCH}"), &bad],
    );
    let packs = world.packs();

    let err = collect(&cli, &world.channel(), &world.request())
        .await
        .expect_err("the bad commit is refused");
    match &err {
        CollectRefusal::FsckFailed(sentence) => assert!(sentence.contains("fsck"), "{sentence}"),
        other => panic!("expected FsckFailed, got {other:?}"),
    }
    assert!(!has_commit(&world.host, &bad).expect("lookup"));
    assert_eq!(world.packs(), packs, "no pack was kept");
    assert_eq!(world.local(), None);
    world.assert_clean();
}

#[tokio::test]
async fn a_bundle_over_the_cap_is_refused() {
    let Some((cli, world)) = world().await else {
        return;
    };
    world.commit_in_clone(&[("a.txt", "a\n")], "a");
    let mut req = world.request();
    req.cap = 64;

    let err = collect(&cli, &world.channel(), &req)
        .await
        .expect_err("over the cap");
    assert!(
        matches!(
            &err,
            CollectRefusal::Channel(ChannelError::OverCap { cap: 64, .. })
        ),
        "{err:?}"
    );
    world.assert_clean();

    let channel = canned_bundle(&world, vec![b'x'; 65]);
    let err = collect(&cli, &channel, &req)
        .await
        .expect_err("a canned answer over the cap");
    assert!(
        matches!(&err, CollectRefusal::Channel(ChannelError::OverCap { .. })),
        "{err:?}"
    );
    assert_eq!(world.local(), None);
    world.assert_clean();
}

#[tokio::test]
async fn an_unreachable_target_is_refused_with_the_channel_sentence() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let err = collect(&cli, &world.channel().unreachable(), &world.request())
        .await
        .expect_err("unreachable");
    let CollectRefusal::Channel(inner) = &err else {
        panic!("expected a channel refusal, got {err:?}");
    };
    assert!(matches!(inner, ChannelError::Unreachable(_)), "{inner:?}");
    assert_eq!(err.to_string(), inner.to_string());
    world.assert_clean();
}

/// R1b M4: a collect writes no temp ref, and leaves one an earlier build wrote to the sweep.
#[tokio::test]
async fn a_leftover_temp_ref_is_left_to_the_sweep() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let tip = world.commit_in_clone(&[("a.txt", "a\n")], "a");
    let leftover = format!("{TEMP_REF_PREFIX}earlier/build");
    create_ref(&world.host, &leftover, &world.base).expect("a leftover");

    let outcome = collect(&cli, &world.channel(), &world.request()).await;
    assert_eq!(outcome.expect("collect"), CollectOutcome::Created { tip });
    assert_eq!(sweep_temp_refs(&world.host).expect("the sweep"), 1);
    world.assert_clean();
}

#[tokio::test]
async fn a_bad_request_is_refused_before_anything_runs() {
    let Some((cli, world)) = world().await else {
        return;
    };
    world.commit_in_clone(&[("a.txt", "a\n")], "a");
    let channel = world.channel().unreachable();
    type Edit = Box<dyn Fn(&mut CollectRequest)>;
    let edits: Vec<(&str, Edit)> = vec![
        (
            "a dash dir",
            Box::new(|req| req.target_dir = "-x".to_owned()),
        ),
        (
            "an empty dir",
            Box::new(|req| req.target_dir = String::new()),
        ),
        (
            "a bad run branch",
            Box::new(|req| req.run_branch = "a..b".to_owned()),
        ),
        (
            "a bad local branch",
            Box::new(|req| req.local_branch = "-f".to_owned()),
        ),
        ("a bad base", Box::new(|req| req.base = "HEAD".to_owned())),
        (
            "a relative scratch",
            Box::new(|req| req.scratch = PathBuf::from("scratch")),
        ),
    ];
    for (what, edit) in edits {
        let mut req = world.request();
        edit(&mut req);
        let err = collect(&cli, &channel, &req).await.expect_err(what);
        assert!(
            matches!(&err, CollectRefusal::Git(IsolateError::Refused(_))),
            "{what}: {err:?}"
        );
    }
    world.assert_clean();
}

/// Plan P4: a hostile clone (`core.fsmonitor`, `uploadpack.packObjectsHook`, `core.hooksPath`
/// with hooks, an `[include]`) drops a sentinel only when [`STAND_IN_MARKER`] is unset. The
/// target may run them; this box never does.
#[cfg(unix)]
#[tokio::test]
async fn a_hostile_clone_never_runs_on_this_box() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let tip = world.commit_in_clone(&[("a.txt", "a\n")], "a");

    let sentinel = world.root.path().join("sentinel");
    std::fs::create_dir_all(&sentinel).expect("the sentinel directory");
    let tools = world.root.path().join("hostile");
    let hooks = tools.join("hooks");
    std::fs::create_dir_all(&hooks).expect("the hooks directory");
    let script = |name: &str| {
        format!(
            "#!/bin/sh\nif [ -n \"${STAND_IN_MARKER}\" ]; then\n  touch '{dir}/on-target-{name}'\nelse\n  touch '{dir}/{name}'\nfi\nexit 0\n",
            dir = sentinel.display(),
        )
    };
    for hook in [
        "post-checkout",
        "pre-push",
        "reference-transaction",
        "post-index-change",
        "pre-auto-gc",
    ] {
        write_executable(&hooks.join(hook), script(hook));
    }
    let fsmonitor = tools.join("fsmonitor");
    write_executable(&fsmonitor, script("fsmonitor"));
    let pack_hook = tools.join("pack-objects-hook");
    write_executable(&pack_hook, script("pack-objects-hook"));
    let included = tools.join("included.config");
    std::fs::write(
        &included,
        format!("[core]\n\tfsmonitor = {}\n", fsmonitor.display()),
    )
    .expect("the included config");
    let config = world.clone_dir().join(".git/config");
    let mut text = std::fs::read_to_string(&config).expect("the clone's config");
    text.push_str(&format!(
        "[core]\n\thooksPath = {}\n[uploadpack]\n\tpackObjectsHook = {}\n[include]\n\tpath = {}\n",
        hooks.display(),
        pack_hook.display(),
        included.display(),
    ));
    std::fs::write(&config, text).expect("the hostile config");

    let channel = world.channel();
    // The fixture is live: a target-side status runs the clone's fsmonitor.
    let status = htui_agent::target_channel::ExecSpec {
        verb: "status",
        argv: ["git", "-C", CLONE, "status", "--porcelain"]
            .map(str::to_owned)
            .to_vec(),
        stdin: htui_agent::target_channel::StdinFrom::Null,
        stdout: htui_agent::target_channel::StdoutTo::Tail,
        timeout: std::time::Duration::from_secs(60),
    };
    use htui_agent::target_channel::TargetChannel as _;
    channel.exec(&status).await.expect("status runs");
    assert!(
        sentinel.join("on-target-fsmonitor").exists(),
        "the fixture is live: the target ran its own fsmonitor"
    );

    let outcome = collect(&cli, &channel, &world.request()).await;
    assert_eq!(outcome.expect("collect"), CollectOutcome::Created { tip });

    let fired: Vec<String> = std::fs::read_dir(&sentinel)
        .expect("the sentinel directory reads")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| !name.starts_with("on-target-"))
        .collect();
    assert!(fired.is_empty(), "this box ran target code: {fired:?}");
    world.assert_clean();
}

/// R1b M1: a target may answer `rev-parse` with a tree or a tag and ship a bundle that matches;
/// the tip must be a commit before any branch is pointed at it.
#[tokio::test]
async fn a_tip_that_is_not_a_commit_is_refused() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let clone = world.clone_dir();
    let commit = world.commit_in_clone(&[("a.txt", "a\n")], "a");
    world.stdout(&clone, &["tag", "-a", "-m", "a tag", "t1", &commit]);
    let tag = world.stdout(&clone, &["rev-parse", "refs/tags/t1"]);
    let tree = world.stdout(&clone, &["rev-parse", &format!("{commit}^{{tree}}")]);
    let base_ref = format!("^{}", world.base);

    let mut tag_objects = world.objects_of(&clone, &[&commit, &base_ref]);
    tag_objects.push(tag.clone());
    let tree_objects: Vec<String> = world
        .objects_of(&clone, &[&commit, &base_ref])
        .into_iter()
        .filter(|object| *object != commit)
        .collect();
    let packs = world.packs();
    for (what, tip, objects) in [
        ("a tree", &tree, tree_objects),
        ("a tag", &tag, tag_objects),
    ] {
        let bytes = world.crafted_bundle(tip, &world.pack_of(&clone, &objects));
        let err = collect(
            &cli,
            &canned_tip_and_bundle(&world, tip, bytes),
            &world.request(),
        )
        .await
        .expect_err(what);
        assert!(
            matches!(&err, CollectRefusal::NotACommit { tip: got } if got == tip),
            "{what}: {err:?}"
        );
        assert_eq!(err.to_string(), not_a_commit(tip), "{what}");
        assert_eq!(world.local(), None, "{what}: nothing was created");
        assert_eq!(world.packs(), packs, "{what}: no pack was kept");
        world.assert_clean();
    }
}

/// R1b M4: a tip `index-pack --strict` takes (its parent is here) whose history is not complete
/// (the parent's own parent is missing) is refused, and nothing it brought is kept.
#[tokio::test]
async fn a_disconnected_tip_is_refused_and_keeps_nothing() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let clone = world.clone_dir();
    let tree = world.stdout(
        &world.host,
        &["rev-parse", &format!("{}^{{tree}}", world.base)],
    );
    let write_commit = |dir: &Path, parent: &str, name: &str| -> String {
        let text = world.root.path().join(name);
        std::fs::write(
            &text,
            format!(
                "tree {tree}\nparent {parent}\nauthor A <a@localhost> 1600000000 +0000\n\
                 committer A <a@localhost> 1600000000 +0000\n\n{name}\n"
            ),
        )
        .expect("the commit text");
        world.stdout(
            dir,
            &[
                "hash-object",
                "--literally",
                "-t",
                "commit",
                "-w",
                text.to_str().expect("utf-8"),
            ],
        )
    };
    let orphan = write_commit(&world.host, &"1".repeat(40), "orphan");
    let tip = write_commit(&clone, &orphan, "tip");
    let bytes = world.crafted_bundle(&tip, &world.pack_of(&clone, std::slice::from_ref(&tip)));
    let packs = world.packs();

    let err = collect(
        &cli,
        &canned_tip_and_bundle(&world, &tip, bytes),
        &world.request(),
    )
    .await
    .expect_err("a disconnected tip");
    assert!(
        matches!(&err, CollectRefusal::NotConnected { tip: got } if *got == tip),
        "{err:?}"
    );
    assert!(!has_commit(&world.host, &tip).expect("lookup"));
    assert_eq!(world.packs(), packs, "no pack was kept");
    assert_eq!(world.local(), None);
    world.assert_clean();
}

/// R1b M2: a collect dropped mid-way (here its `before_cas` panics, as a cancel or a timeout
/// would drop it) leaves no temp ref, no quarantine and no pack behind.
#[tokio::test]
async fn a_collect_dropped_mid_way_leaves_nothing_behind() {
    let Some((cli, world)) = world().await else {
        return;
    };
    world.commit_in_clone(&[("a.txt", "a\n")], "a");
    let packs = world.packs();
    let (channel, req) = (world.channel(), world.request());
    let joined = tokio::spawn(async move {
        collect_racing(&cli, &channel, &req, &|| {
            panic!("the collect is dropped here")
        })
        .await
    })
    .await;
    assert!(joined.is_err_and(|err| err.is_panic()));
    assert_eq!(world.local(), None);
    assert_eq!(world.packs(), packs, "no pack was kept");
    world.assert_clean();
}

/// R1b M3: a consent holds over the local tip the person saw; a local commit since asks again.
#[tokio::test]
async fn a_consent_is_bound_to_the_local_tip_it_was_given_over() {
    let Some((cli, world)) = world().await else {
        return;
    };
    let channel = world.channel();
    let first = world.commit_in_clone(&[("a.txt", "a\n")], "a");
    collect(&cli, &channel, &world.request())
        .await
        .expect("the first collect");
    world.reset_clone(&world.base);
    let rewritten = world.commit_in_clone(&[("c.txt", "c\n")], "c");

    // The person consents over `first`; then a local commit moves the branch to `moved`.
    let mut req = world.request();
    req.consent = consent(&rewritten, Some(&first), true, &[]);
    let moved = htui_orch::isolate::git::testkit::commit_file(&world.host, "h", "h\n", "h");
    htui_orch::isolate::git::move_branch(&world.host, RUN_BRANCH, &first, &moved)
        .expect("the local move");

    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::NeedsConsent {
            tip: rewritten.clone(),
            rewritten_from: Some(moved.clone()),
            config_paths: Vec::new(),
        }
    );
    assert_eq!(world.local(), Some(moved.clone()), "nothing moved");
    world.assert_clean();

    req.consent = consent(&rewritten, Some(&moved), true, &[]);
    let outcome = collect(&cli, &channel, &req).await;
    assert_eq!(
        outcome.expect("collect"),
        CollectOutcome::Forced {
            from: moved,
            tip: rewritten.clone(),
        }
    );
    assert_eq!(world.local(), Some(rewritten));
    world.assert_clean();
}

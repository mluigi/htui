//! MOD-132 M1 T3: [`seed`] makes a target's clone from a bundle made on this box (plan P5), over
//! the test-only [`StandInChannel`].
//!
//! Every case opens with [`skip_without_git`] and passes on a box without one.

use std::path::{Path, PathBuf};
use std::process::Output;

use htui_agent::target_channel::{Canned, STAND_IN_MARKER, StandInChannel};
use htui_orch::isolate::git::SCRUBBED_ENV;
use htui_orch::isolate::git::testkit::{commit_file, repo_with_one_commit};
use htui_orch::skip_without_git;
use htui_orch::target::seed::{
    PUSH_REFUSED, PrepareRefusal, SeedRequest, Seeded, credential_host, credential_in_url,
    no_run_branch, occupied, seed, seeded_other,
};

use crate::write_executable::write_executable;

/// One test's directories: this box's checkout, the scratch the bundle goes to, the stand-in's
/// home, and an empty home the oracle `git` runs under.
struct World {
    _root: tempfile::TempDir,
    host: PathBuf,
    scratch: PathBuf,
    home: PathBuf,
    oracle_home: PathBuf,
    base: String,
}

fn world() -> World {
    let root = tempfile::tempdir().expect("a temporary directory");
    let host = root.path().join("host");
    let scratch = root.path().join("scratch");
    let home = root.path().join("target-home");
    let oracle_home = root.path().join("oracle-home");
    for dir in [&scratch, &home, &oracle_home] {
        std::fs::create_dir_all(dir).expect("a directory");
    }
    let base = repo_with_one_commit(&host);
    World {
        host,
        scratch,
        home,
        oracle_home,
        base,
        _root: root,
    }
}

impl World {
    /// The full name of the branch `HEAD` names in this box's checkout.
    fn base_ref(&self) -> String {
        let out = self.git(&self.host, &["symbolic-ref", "HEAD"]);
        assert!(out.status.success(), "symbolic-ref: {out:?}");
        String::from_utf8(out.stdout)
            .expect("utf-8")
            .trim()
            .to_owned()
    }

    fn request(&self, target_dir: &str) -> SeedRequest {
        SeedRequest {
            repo: self.host.clone(),
            base_ref: self.base_ref(),
            target_dir: target_dir.to_owned(),
            run_branch: "htui/run/one".to_owned(),
            remote_url: Some(self.host.display().to_string()),
            scratch: self.scratch.clone(),
        }
    }

    /// The oracle: `git` in `dir`, with the location variables scrubbed and an empty home, so
    /// it reads the clone's own configuration and nothing of this box's.
    fn git(&self, dir: &Path, args: &[&str]) -> Output {
        let mut command = std::process::Command::new("git");
        command.arg("-C").arg(dir).args(args);
        for key in SCRUBBED_ENV {
            command.env_remove(key);
        }
        command
            .env("HOME", &self.oracle_home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("LC_ALL", "C")
            .stdin(std::process::Stdio::null())
            .output()
            .expect("git runs")
    }

    fn stdout(&self, dir: &Path, args: &[&str]) -> String {
        let out = self.git(dir, args);
        assert!(out.status.success(), "git {args:?} in {dir:?}: {out:?}");
        String::from_utf8(out.stdout)
            .expect("utf-8")
            .trim()
            .to_owned()
    }

    fn scratch_is_empty(&self) -> bool {
        std::fs::read_dir(&self.scratch)
            .expect("the scratch reads")
            .next()
            .is_none()
    }
}

#[tokio::test]
async fn seeds_a_clone_on_the_run_branch_that_cannot_push() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let channel = StandInChannel::new(&world.home);
    let req = world.request("work/clone");

    let seeded = seed(&cli, &channel, &req).await.expect("the seed succeeds");
    assert_eq!(
        seeded,
        Seeded {
            base: world.base.clone(),
            reused: false,
        }
    );

    let clone = world.home.join("work/clone");
    assert_eq!(
        world.stdout(&clone, &["cat-file", "-t", &world.base]),
        "commit"
    );
    assert_eq!(
        world.stdout(&clone, &["symbolic-ref", "HEAD"]),
        "refs/heads/htui/run/one"
    );
    assert_eq!(world.stdout(&clone, &["rev-parse", "HEAD"]), world.base);
    assert_eq!(world.stdout(&clone, &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(clone.join("f")).expect("the base's file is checked out"),
        "first\n"
    );
    assert_eq!(
        world.stdout(&clone, &["config", "--get", "remote.origin.url"]),
        world.host.display().to_string()
    );
    assert_eq!(
        world.stdout(&clone, &["config", "--get", "remote.origin.pushurl"]),
        PUSH_REFUSED
    );
    assert_eq!(
        world.stdout(&clone, &["config", "--get", "htui.seededBase"]),
        world.base
    );

    let push = world.git(&clone, &["push", "origin", "htui/run/one"]);
    assert!(!push.status.success(), "a push from the clone fails");
    assert!(
        world
            .git(
                &world.host,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "refs/heads/htui/run/one"
                ]
            )
            .stdout
            .is_empty(),
        "nothing reached this box's checkout"
    );

    let credentials = world.git(&clone, &["config", "--get-regexp", "^credential"]);
    assert_eq!(credentials.status.code(), Some(1), "no credential config");
    for path in [
        world.home.join(".git-credentials"),
        clone.join(".git-credentials"),
        clone.join(".git/credentials"),
    ] {
        assert!(!path.exists(), "{} was written", path.display());
    }

    assert!(world.scratch_is_empty(), "the quarantine bundle is removed");
}

#[tokio::test]
async fn a_reseed_reuses_a_different_base_or_an_occupied_dir_refuses() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let channel = StandInChannel::new(&world.home);
    let req = world.request("clone");

    seed(&cli, &channel, &req).await.expect("the first seed");
    let again = seed(&cli, &channel, &req).await.expect("the re-seed");
    assert_eq!(
        again,
        Seeded {
            base: world.base.clone(),
            reused: true,
        }
    );
    assert!(world.scratch_is_empty());

    let moved = commit_file(&world.host, "g", "second\n", "two");
    assert_ne!(moved, world.base);
    let err = seed(&cli, &channel, &req)
        .await
        .expect_err("a different base is refused");
    match &err {
        PrepareRefusal::SeededOther { dir, base } => {
            assert_eq!(dir, "clone");
            assert_eq!(base, &world.base);
        }
        other => panic!("expected SeededOther, got {other:?}"),
    }
    assert_eq!(err.to_string(), seeded_other("clone", &world.base));
    assert!(world.scratch_is_empty(), "removed after a refusal too");

    std::fs::create_dir_all(world.home.join("busy")).expect("a directory");
    std::fs::write(world.home.join("busy/notes.txt"), "mine\n").expect("a file");
    let err = seed(&cli, &channel, &world.request("busy"))
        .await
        .expect_err("an occupied dir is refused");
    assert!(
        matches!(&err, PrepareRefusal::Occupied { dir } if dir == "busy"),
        "{err:?}"
    );
    assert_eq!(err.to_string(), occupied("busy"));
    assert!(
        !world.home.join("busy/.git").exists(),
        "nothing was written into it"
    );

    // A repository without the seed's marker is somebody else's too.
    let other = world.home.join("other-repo");
    repo_with_one_commit(&other);
    let err = seed(&cli, &channel, &world.request("other-repo"))
        .await
        .expect_err("a foreign repository is refused");
    assert!(
        matches!(&err, PrepareRefusal::Occupied { dir } if dir == "other-repo"),
        "{err:?}"
    );

    // An empty directory is not occupied.
    std::fs::create_dir_all(world.home.join("empty")).expect("a directory");
    seed(&cli, &channel, &world.request("empty"))
        .await
        .expect("an empty dir is seeded");
    assert!(world.scratch_is_empty());
}

#[tokio::test]
async fn an_unreachable_target_refuses_with_the_channel_sentence() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let channel = StandInChannel::new(&world.home).unreachable();

    let err = seed(&cli, &channel, &world.request("clone"))
        .await
        .expect_err("an unreachable target refuses");
    let PrepareRefusal::Channel(inner) = &err else {
        panic!("expected a channel refusal, got {err:?}");
    };
    assert_eq!(err.to_string(), inner.to_string());
    assert!(err.to_string().contains("is unreachable"), "{err}");
    assert!(world.scratch_is_empty(), "the quarantine bundle is removed");
    assert!(!world.home.join("clone").exists());
}

#[tokio::test]
async fn a_base_ref_this_box_lacks_refuses_and_leaves_no_bundle() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let channel = StandInChannel::new(&world.home);
    let mut req = world.request("clone");
    req.base_ref = "refs/heads/no-such-branch".to_owned();

    let err = seed(&cli, &channel, &req)
        .await
        .expect_err("a missing base refuses");
    assert!(matches!(err, PrepareRefusal::Git(_)), "{err:?}");
    assert!(world.scratch_is_empty());
    assert!(
        !world.home.join("clone").exists(),
        "the target was not touched"
    );
}

#[tokio::test]
async fn a_run_branch_that_is_not_a_branch_name_refuses_before_anything_runs() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let channel = StandInChannel::new(&world.home);
    for bad in ["", "-f", "a..b", "has space", "refs/heads/x"] {
        let mut req = world.request("clone");
        req.run_branch = bad.to_owned();
        let err = seed(&cli, &channel, &req)
            .await
            .expect_err("a bad run branch refuses");
        assert!(matches!(err, PrepareRefusal::Git(_)), "{bad:?}: {err:?}");
    }
    assert!(world.scratch_is_empty());
    assert!(!world.home.join("clone").exists());
}

/// Plan P4: a hostile `~/.gitconfig` in the stand-in's home installs a hook that drops a
/// sentinel only when [`STAND_IN_MARKER`] is unset. It fires on the target (the proof that the
/// fixture is live) and never on this box.
#[cfg(unix)]
#[tokio::test]
async fn a_hostile_target_gitconfig_never_reaches_this_box() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let sentinel = world.home.parent().expect("a root").join("sentinel");
    std::fs::create_dir_all(&sentinel).expect("the sentinel directory");
    let hooks = world.home.join("hooks");
    std::fs::create_dir_all(&hooks).expect("the hooks directory");
    for hook in [
        "post-checkout",
        "pre-push",
        "reference-transaction",
        "post-index-change",
    ] {
        write_executable(
            &hooks.join(hook),
            format!(
                "#!/bin/sh\nif [ -n \"${STAND_IN_MARKER}\" ]; then\n  touch '{dir}/on-target-{hook}'\nelse\n  touch '{dir}/{hook}'\nfi\nexit 0\n",
                dir = sentinel.display(),
            ),
        );
    }
    std::fs::write(
        world.home.join(".gitconfig"),
        format!(
            "[core]\n\thooksPath = {}\n[credential]\n\thelper = !touch '{}/credential-helper'\n",
            hooks.display(),
            sentinel.display()
        ),
    )
    .expect("the hostile gitconfig");

    let channel = StandInChannel::new(&world.home);
    seed(&cli, &channel, &world.request("clone"))
        .await
        .expect("the seed succeeds under a hostile target config");
    seed(&cli, &channel, &world.request("clone"))
        .await
        .expect("and the re-seed");

    assert!(
        sentinel.join("on-target-post-checkout").exists(),
        "the fixture is live: the target ran its own hook"
    );
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
    assert!(world.scratch_is_empty());
}

/// The seed never leaves the bundle behind in a scratch that already held other files, and
/// leaves those alone.
#[tokio::test]
async fn the_scratch_keeps_only_what_it_held_before() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    std::fs::write(world.scratch.join("keep.txt"), "mine\n").expect("a file");
    let channel = StandInChannel::new(&world.home);
    seed(&cli, &channel, &world.request("clone"))
        .await
        .expect("the seed succeeds");
    let names: Vec<_> = std::fs::read_dir(&world.scratch)
        .expect("the scratch reads")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("keep.txt")]);
}

/// R1b M5: a `remote_url` with a password or a token is refused before anything runs, by its host
/// alone; an ssh user without a password is no credential.
#[tokio::test]
async fn a_remote_url_with_a_credential_is_refused() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let channel = StandInChannel::new(&world.home).unreachable();
    for (url, host) in [
        (
            "https://user:s3cret-token@forge.example:8443/a/b.git",
            "forge.example:8443",
        ),
        (
            "https://s3cret-token@forge.example/a/b.git",
            "forge.example",
        ),
        ("HTTP://s3cret-token@forge.example", "forge.example"),
        (
            "ssh://git:s3cret-token@forge.example/a/b.git",
            "forge.example",
        ),
    ] {
        let mut req = world.request("clone");
        req.remote_url = Some(url.to_owned());
        let err = seed(&cli, &channel, &req).await.expect_err(url);
        assert!(
            matches!(&err, PrepareRefusal::CredentialInUrl { host: got } if got == host),
            "{url}: {err:?}"
        );
        assert_eq!(err.to_string(), credential_in_url(host));
        assert!(!err.to_string().contains("s3cret"), "{err}");
    }
    assert!(world.scratch_is_empty());
    assert!(!world.home.join("clone").exists());

    for url in [
        "git@forge.example:a/b.git",
        "ssh://git@forge.example/a/b.git",
        "https://forge.example/a/b.git",
        "/srv/git/a.git",
        "file:///srv/git/a.git",
    ] {
        assert_eq!(credential_host(url), None, "{url}");
    }
    let mut req = world.request("clone");
    req.remote_url = Some("ssh://git@forge.example/a/b.git".to_owned());
    seed(&cli, &StandInChannel::new(&world.home), &req)
        .await
        .expect("an ssh user without a password is seeded");
    assert_eq!(
        world.stdout(
            &world.home.join("clone"),
            &["config", "--get", "remote.origin.url"]
        ),
        "ssh://git@forge.example/a/b.git"
    );
}

/// R1b L4: a clone seeded at this base is reused only when it has the run branch; without it the
/// directory is refused, never seeded again over.
#[tokio::test]
async fn a_seeded_clone_without_the_run_branch_is_refused() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let channel = StandInChannel::new(&world.home);
    seed(&cli, &channel, &world.request("clone"))
        .await
        .expect("the first seed");
    let clone = world.home.join("clone");
    let head = world.stdout(&clone, &["rev-parse", "HEAD"]);

    let mut req = world.request("clone");
    req.run_branch = "htui/run/two".to_owned();
    let err = seed(&cli, &channel, &req)
        .await
        .expect_err("another run branch is refused");
    assert!(
        matches!(&err, PrepareRefusal::NoRunBranch { dir, branch }
            if dir == "clone" && branch == "htui/run/two"),
        "{err:?}"
    );
    assert_eq!(err.to_string(), no_run_branch("clone", "htui/run/two"));
    assert!(err.to_string().contains("clone") && err.to_string().contains("htui/run/two"));
    assert_eq!(world.stdout(&clone, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        world
            .git(
                &clone,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "refs/heads/htui/run/two"
                ]
            )
            .status
            .code(),
        Some(1),
        "nothing was seeded over it"
    );
    assert!(world.scratch_is_empty());
}

/// R1b L8: the seed bundle is readable by this user only by the time the channel reads it.
#[cfg(unix)]
#[tokio::test]
async fn the_seed_bundle_is_private_when_the_channel_reads_it() {
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::{Arc, Mutex};

    let Some(cli) = skip_without_git!() else {
        return;
    };
    let world = world();
    let modes = Arc::new(Mutex::new(Vec::new()));
    let (seen, scratch) = (Arc::clone(&modes), world.scratch.clone());
    let channel =
        StandInChannel::new(&world.home).canned(move |argv: &[String]| -> Option<Canned> {
            if argv.iter().any(|arg| arg == "index-pack") {
                for entry in std::fs::read_dir(&scratch).expect("the scratch reads") {
                    let mode = entry
                        .expect("an entry")
                        .metadata()
                        .expect("metadata")
                        .permissions()
                        .mode();
                    seen.lock().expect("the lock").push(mode & 0o777);
                }
            }
            None
        });
    seed(&cli, &channel, &world.request("clone"))
        .await
        .expect("the seed succeeds");
    assert_eq!(*modes.lock().expect("the lock"), vec![0o600]);
    assert!(world.scratch_is_empty());
}

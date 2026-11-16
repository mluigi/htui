//! MOD-116 M3 D8: the git helpers a workspace's new project is set up with — `Cli::clone_repo`,
//! `Cli::init_with_empty_commit` and `repo_facts` — over real `git`, against local bare
//! repositories in temp dirs (no network, no credentials).
//!
//! Every case opens with [`skip_without_git`] and passes on a box without a usable `git`.

use std::ffi::OsStr;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use htui_orch::isolate::IsolateError;
use htui_orch::isolate::git::testkit::{empty_repo, repo_with_one_commit};
use htui_orch::isolate::git::{Cli, RepoFacts, enclosing_repository, head, repo_facts};
use htui_orch::skip_without_git;

/// `git <args>` in `cwd`, failing the case on a non-zero exit; its stdout, trimmed.
async fn git_out(git: &Cli, cwd: &Path, args: &[&str]) -> String {
    let argv: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    let exited = git.run("oracle", cwd, &argv, &[]).await.expect("git runs");
    assert!(exited.ok(), "git {args:?} failed: {}", exited.stderr);
    exited.stdout.trim().to_owned()
}

/// An empty bare repository at `<dir>/<name>.git` whose `HEAD` names `main`.
async fn bare_main(git: &Cli, dir: &Path, name: &str) -> PathBuf {
    let remote = dir.join(format!("{name}.git"));
    git_out(
        git,
        dir,
        &[
            "init",
            "--quiet",
            "--bare",
            "--initial-branch=main",
            &remote.to_string_lossy(),
        ],
    )
    .await;
    remote
}

/// A bare repository at `<dir>/<name>.git` whose `HEAD` names `main` and whose `main` is `from`'s
/// `HEAD`. Unlike `gix_isolator.rs`'s `bare_remote`, its `HEAD` names the branch the push lands,
/// so a clone checks it out.
async fn bare_remote(git: &Cli, dir: &Path, name: &str, from: &Path) -> PathBuf {
    let remote = bare_main(git, dir, name).await;
    git_out(
        git,
        from,
        &[
            "push",
            "--quiet",
            "--",
            &remote.to_string_lossy(),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    remote
}

/// The reason of a `Refused`, failing the case on anything else.
fn refused(result: Result<(), IsolateError>) -> String {
    match result {
        Err(IsolateError::Refused(reason)) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_clone_of_a_local_bare_repo_lands_at_dest_with_its_head() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let source = tmp.path().join("source");
    std::fs::create_dir(&source).expect("the source dir");
    let commit = repo_with_one_commit(&source);
    let remote = bare_remote(&git, tmp.path(), "api", &source).await;
    let projects = tmp.path().join("projects");
    std::fs::create_dir(&projects).expect("the projects dir");
    let dest = projects.join("api");

    git.clone_repo(&remote.to_string_lossy(), &dest)
        .await
        .expect("the clone succeeds");

    assert_eq!(head(&dest).expect("HEAD reads"), commit);
    let facts = repo_facts(&dest)
        .expect("the facts read")
        .expect("a repository");
    assert_eq!(
        facts,
        RepoFacts {
            branch: Some("main".to_owned()),
            origin: Some(remote.to_string_lossy().into_owned()),
            head: Some(commit),
            bare: false,
            workdir: Some(dest.clone()),
        }
    );
    assert!(dest.join("f").is_file(), "the tree is checked out");
}

#[tokio::test]
async fn a_clone_of_a_missing_url_is_refused_without_a_prompt_and_leaves_no_dest() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let url = tmp.path().join("nothing.git");
    let dest = tmp.path().join("api");

    let reason = refused(git.clone_repo(&url.to_string_lossy(), &dest).await);

    assert!(reason.starts_with("git clone: "), "{reason}");
    assert!(!dest.exists(), "nothing is left at {}", dest.display());
}

#[tokio::test]
async fn a_clone_url_s_userinfo_never_reaches_the_message() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let dest = tmp.path().join("api");

    let started = std::time::Instant::now();
    let reason = refused(
        git.clone_repo("https://user:secret@127.0.0.1:9/x.git", &dest)
            .await,
    );

    assert!(reason.starts_with("git clone: "), "{reason}");
    assert!(!reason.contains("secret"), "{reason}");
    assert!(!reason.contains("user:"), "{reason}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(60),
        "a refused connection answers at once, never a prompt: {:?}",
        started.elapsed()
    );
    assert!(!dest.exists(), "nothing is left at {}", dest.display());
}

#[tokio::test]
async fn an_empty_remote_clones_with_an_unborn_head() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let remote = bare_main(&git, tmp.path(), "empty").await;
    let dest = tmp.path().join("api");

    git.clone_repo(&remote.to_string_lossy(), &dest)
        .await
        .expect("an empty repository clones with exit 0");

    assert!(dest.is_dir());
    let facts = repo_facts(&dest)
        .expect("the facts read")
        .expect("a repository");
    assert_eq!(facts.branch.as_deref(), Some("main"));
    assert_eq!(facts.head, None, "nothing is committed");
    assert!(!facts.bare);
}

#[tokio::test]
async fn a_new_repo_has_one_empty_commit_and_nothing_in_its_tree() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let dir = tmp.path().join("api");
    std::fs::create_dir(&dir).expect("the folder");

    // Under the person's own configuration, as production runs: GIT_CONFIG_GLOBAL is not touched.
    let commit = git
        .init_with_empty_commit(&dir, "main")
        .await
        .expect("the repository is made");

    assert_eq!(head(&dir).expect("HEAD reads"), commit);
    let log = git_out(&git, &dir, &["log", "--oneline"]).await;
    assert_eq!(log.lines().count(), 1, "{log}");
    assert_eq!(
        git_out(
            &git,
            &dir,
            &["log", "-1", "--format=%an <%ae>|%cn <%ce>|%s"]
        )
        .await,
        "htui <htui@localhost>|htui <htui@localhost>|Initial commit"
    );
    assert_eq!(git_out(&git, &dir, &["ls-tree", "-r", "HEAD"]).await, "");
    assert_eq!(git_out(&git, &dir, &["status", "--porcelain"]).await, "");
    let names: Vec<_> = std::fs::read_dir(&dir)
        .expect("the folder lists")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    assert_eq!(names, [".git"], "nothing but `.git` is written (R-ID-4)");
    let facts = repo_facts(&dir).expect("the facts read").expect("a repo");
    assert_eq!(facts.branch.as_deref(), Some("main"));
    assert_eq!(facts.origin, None);
}

#[tokio::test]
async fn repo_facts_reads_branch_origin_and_head() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let commit = repo_with_one_commit(tmp.path());
    git_out(
        &git,
        tmp.path(),
        &["remote", "add", "origin", "https://u:t@h/x"],
    )
    .await;

    let facts = repo_facts(tmp.path())
        .expect("the facts read")
        .expect("a repository");

    assert_eq!(facts.branch.as_deref(), Some("main"));
    assert_eq!(
        facts.origin.as_deref(),
        Some("https://u:t@h/x"),
        "the caller redacts, not repo_facts"
    );
    assert_eq!(facts.head, Some(commit));
    assert!(!facts.bare);
}

#[test]
fn repo_facts_of_a_plain_folder_is_none_and_of_a_subfolder_too() {
    let tmp = tempfile::tempdir().expect("a temp dir");
    let plain = tmp.path().join("plain");
    std::fs::create_dir(&plain).expect("a plain folder");
    assert_eq!(repo_facts(&plain).expect("a plain folder reads"), None);

    let repo = tmp.path().join("repo");
    std::fs::create_dir(&repo).expect("the repo folder");
    repo_with_one_commit(&repo);
    let sub = repo.join("sub");
    std::fs::create_dir(&sub).expect("a subfolder");
    assert_eq!(
        repo_facts(&sub).expect("a subfolder reads"),
        None,
        "a repository is never discovered upward"
    );
}

#[test]
fn repo_facts_of_an_unborn_repo_has_no_head_but_a_branch() {
    let tmp = tempfile::tempdir().expect("a temp dir");
    empty_repo(tmp.path());

    let facts = repo_facts(tmp.path())
        .expect("the facts read")
        .expect("a repository");

    assert!(facts.branch.is_some(), "{facts:?}");
    assert_eq!(facts.head, None);
    assert_eq!(facts.origin, None);
    assert!(!facts.bare);
}

#[tokio::test]
async fn repo_facts_says_bare() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let remote = bare_main(&git, tmp.path(), "bare").await;

    let facts = repo_facts(&remote)
        .expect("the facts read")
        .expect("a repository");

    assert!(facts.bare, "{facts:?}");
    assert_eq!(facts.branch.as_deref(), Some("main"));
    assert_eq!(facts.head, None);
}

/// `git` behind a wrapper that hands the verb `name=value` as an inherited variable would be: set
/// only when the verb's own environment does not already set it (an htui `env` wins, as it does
/// over the parent's).
fn git_inheriting(git: &Cli, dir: &Path, name: &str, value: &str) -> Cli {
    let wrapper = dir.join(format!("git-with-{}", name.to_lowercase()));
    crate::write_executable::write_executable(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ -z \"${{{name}+x}}\" ]; then {name}='{value}'; export {name}; fi\n\
             exec '{}' \"$@\"\n",
            git.binary().display()
        ),
    );
    Cli::probe(wrapper).expect("the wrapper answers `--version` as git")
}

/// A script that records it ran in `<dir>/<name>.ran` and answers `x` to any question.
fn recording_script(dir: &Path, name: &str) -> (PathBuf, PathBuf) {
    let script = dir.join(name);
    let ran = dir.join(format!("{name}.ran"));
    crate::write_executable::write_executable(
        &script,
        format!("#!/bin/sh\necho \"$@\" >> '{}'\necho x\n", ran.display()),
    );
    (script, ran)
}

/// A local http server that answers every request `401` with a Basic challenge, on its own
/// thread for the rest of the process; the URL of a repository on it.
fn unauthorized_server() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
    let port = listener.local_addr().expect("its address").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut head = Vec::new();
            let mut chunk = [0_u8; 4096];
            while !head.windows(4).any(|window| window == b"\r\n\r\n") {
                match stream.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => head.extend_from_slice(&chunk[..read]),
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"htui\"\r\n\
                  Content-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    format!("http://127.0.0.1:{port}/private.git")
}

/// MOD-116 M3 R1a A1 (D7: htui never prompts): a private https remote on a desktop whose
/// `SSH_ASKPASS` or `GIT_ASKPASS` names a dialog fails at once, and the dialog never opens.
#[tokio::test]
async fn a_clone_that_needs_credentials_never_asks_an_askpass_program() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let url = unauthorized_server();
    for variable in ["SSH_ASKPASS", "GIT_ASKPASS"] {
        let tmp = tempfile::tempdir().expect("a temp dir");
        let (askpass, ran) = recording_script(tmp.path(), "askpass");
        let git = git_inheriting(&git, tmp.path(), variable, &askpass.to_string_lossy());
        let dest = tmp.path().join("api");

        let started = std::time::Instant::now();
        let reason = refused(git.clone_repo(&url, &dest).await);

        assert!(reason.starts_with("git clone: "), "{variable}: {reason}");
        assert!(
            reason.contains("could not read Username"),
            "{variable}: the clone says it had no credentials: {reason}"
        );
        assert!(
            !ran.exists(),
            "{variable} was asked: {:?}",
            std::fs::read_to_string(&ran)
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "{variable}: {:?}",
            started.elapsed()
        );
        assert!(!dest.exists(), "nothing is left at {}", dest.display());
    }
}

/// MOD-116 M3 R1a A5: an inherited `GIT_ALLOW_PROTOCOL` that allows `ext` does not lift the
/// clone's own refusal of it; the transport's command never runs.
#[tokio::test]
async fn an_inherited_allow_protocol_does_not_let_ext_run_a_command() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let git = git_inheriting(&git, tmp.path(), "GIT_ALLOW_PROTOCOL", "ext");
    let marker = tmp.path().join("ran");
    let dest = tmp.path().join("api");

    let url = format!("ext::sh -c touch% {}", marker.display());
    let reason = refused(git.clone_repo(&url, &dest).await);

    assert!(reason.starts_with("git clone: "), "{reason}");
    assert!(reason.contains("transport 'ext' not allowed"), "{reason}");
    assert!(!marker.exists(), "the ext command ran");
    assert!(!dest.exists(), "nothing is left at {}", dest.display());
}

/// MOD-116 M3 R1a A8: the work tree a repository's facts name is the repository's own, whichever
/// of its folders was typed: its top, its `.git` directory, or a linked worktree's git directory
/// inside the main repository's `.git`.
#[tokio::test]
async fn repo_facts_name_the_work_tree_whichever_git_folder_is_typed() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let main = tmp.path().join("main");
    std::fs::create_dir(&main).expect("the main checkout");
    repo_with_one_commit(&main);
    let linked = tmp.path().join("linked");
    git_out(
        &git,
        &main,
        &[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            &linked.to_string_lossy(),
        ],
    )
    .await;
    let canonical = |path: &Path| std::fs::canonicalize(path).expect("the path resolves");
    let workdir = |typed: &Path| {
        repo_facts(typed)
            .expect("the facts read")
            .expect("a repository")
            .workdir
            .map(|workdir| canonical(&workdir))
    };

    assert_eq!(workdir(&main), Some(canonical(&main)));
    assert_eq!(workdir(&main.join(".git")), Some(canonical(&main)));
    assert_eq!(workdir(&linked), Some(canonical(&linked)));
    assert_eq!(
        workdir(&main.join(".git").join("worktrees").join("linked")),
        Some(canonical(&linked)),
        "a linked worktree's git directory names that worktree, never a folder inside `.git`"
    );
}

/// MOD-116 M3 R1a A3 (`R-ID-4`): a folder inside a work tree, inside a `.git` directory or inside
/// a bare repository is enclosed by that repository; a plain folder, and a repository's own top,
/// are not (the top is `repo_facts`' to name).
#[tokio::test]
async fn enclosing_repository_finds_the_repository_a_folder_is_inside() {
    let Some(git) = skip_without_git!() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("a temp dir");
    let repo = tmp.path().join("api");
    std::fs::create_dir(&repo).expect("the repo folder");
    repo_with_one_commit(&repo);
    let deep = repo.join("ws").join("deeper");
    std::fs::create_dir_all(&deep).expect("folders inside the work tree");
    let bare = bare_main(&git, tmp.path(), "bare").await;
    let in_bare = bare.join("ws");
    std::fs::create_dir(&in_bare).expect("a folder inside the bare repository");
    let plain = tmp.path().join("plain").join("ws");
    std::fs::create_dir_all(&plain).expect("a plain folder");

    let enclosing = |path: &Path| enclosing_repository(path).expect("the ancestors read");
    assert_eq!(enclosing(&repo.join("ws")), Some(repo.clone()));
    assert_eq!(enclosing(&deep), Some(repo.clone()));
    assert_eq!(
        enclosing(&repo.join(".git").join("refs")),
        Some(repo.join(".git"))
    );
    assert_eq!(enclosing(&in_bare), Some(bare));
    assert_eq!(enclosing(&plain), None);
    assert_eq!(
        enclosing(&repo),
        None,
        "a repository's own top is not inside one"
    );
}

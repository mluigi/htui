//! MOD-132 M1 T2: the bundle header parser (P7) and the git verbs a seed and a collect are built
//! from: `bundle create`, `bundle verify`, `index-pack`, the connectivity check, and the `gix`
//! ref reads and writes the temp ref uses.
//!
//! Every case that needs `git` opens with [`skip_without_git`] and passes on a box without one.

use std::path::{Path, PathBuf};

use htui_orch::IsolateError;
use htui_orch::isolate::git::testkit::{commit_file, empty_repo, repo_with_one_commit};
use htui_orch::isolate::git::{
    Cli, ObjectQuarantine, SCRUBBED_ENV, create_ref, delete_ref, has_commit, migrate_quarantine,
    objects_dir, ref_target,
};
use htui_orch::skip_without_git;
use htui_orch::target::bundle::{
    BundleHeader, BundleRefusal, HEADER_CAP, parse_header, read_header,
};

const A: &str = "0123456789abcdef0123456789abcdef01234567";
/// The `--max-input-size` the round trips pass: far above any pack they make.
const CAP: u64 = 1 << 30;
const B: &str = "89abcdef0123456789abcdef0123456789abcdef";

/// A header from its lines, terminated by the empty line, with a fake pack after it.
fn bundle(lines: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in lines {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    bytes.push(b'\n');
    bytes.extend_from_slice(b"PACK\0\0\0\x02");
    bytes
}

#[test]
fn parses_a_v2_header_with_commented_prerequisites() {
    let bytes = bundle(&[
        "# v2 git bundle",
        &format!("-{A} the subject of the base commit"),
        &format!("-{B}"),
        &format!("{A} refs/heads/htui/item/one"),
    ]);
    let header = parse_header(&bytes).expect("a v2 header parses");
    assert_eq!(
        header,
        BundleHeader {
            version: 2,
            prerequisites: vec![A.to_owned(), B.to_owned()],
            refs: vec![(A.to_owned(), "refs/heads/htui/item/one".to_owned())],
            pack_offset: u64::try_from(bytes.len() - 8).expect("small"),
        }
    );
    let offset = usize::try_from(header.pack_offset).expect("small");
    assert_eq!(&bytes[offset..offset + 4], b"PACK");
}

#[test]
fn parses_a_v3_header_with_the_sha1_object_format() {
    let bytes = bundle(&[
        "# v3 git bundle",
        "@object-format=sha1",
        &format!("{B} refs/heads/main"),
    ]);
    let header = parse_header(&bytes).expect("a v3 sha1 header parses");
    assert_eq!(header.version, 3);
    assert!(header.prerequisites.is_empty());
    assert_eq!(
        header.refs,
        vec![(B.to_owned(), "refs/heads/main".to_owned())]
    );
}

#[test]
fn refuses_each_class_of_bad_header() {
    let long_hash = format!("{A}0");
    let cases: Vec<(&str, Vec<u8>, BundleRefusal)> = vec![
        (
            "filter",
            bundle(&[
                "# v3 git bundle",
                "@filter=blob:none",
                &format!("{A} refs/heads/x"),
            ]),
            BundleRefusal::Filter("blob:none".to_owned()),
        ),
        (
            "unknown capability",
            bundle(&[
                "# v3 git bundle",
                "@frobnicate=1",
                &format!("{A} refs/heads/x"),
            ]),
            BundleRefusal::Capability("frobnicate=1".to_owned()),
        ),
        (
            "sha256",
            bundle(&[
                "# v3 git bundle",
                "@object-format=sha256",
                &format!("{A} refs/heads/x"),
            ]),
            BundleRefusal::Sha256,
        ),
        (
            "capability in v2",
            bundle(&[
                "# v2 git bundle",
                "@object-format=sha1",
                &format!("{A} refs/heads/x"),
            ]),
            BundleRefusal::MalformedLine(2),
        ),
        (
            "capability after a ref",
            bundle(&[
                "# v3 git bundle",
                &format!("{A} refs/heads/x"),
                "@object-format=sha1",
            ]),
            BundleRefusal::MalformedLine(3),
        ),
        (
            "unknown version",
            bundle(&["# v4 git bundle", &format!("{A} refs/heads/x")]),
            BundleRefusal::UnknownVersion("v4".to_owned()),
        ),
        (
            "garbage",
            b"PK\x03\x04 this is a zip file\n\n".to_vec(),
            BundleRefusal::NotABundle,
        ),
        ("empty", Vec::new(), BundleRefusal::NotABundle),
        (
            "crlf",
            format!("# v2 git bundle\r\n{A} refs/heads/x\r\n\r\nPACK").into_bytes(),
            BundleRefusal::MalformedLine(1),
        ),
        (
            "crlf after the signature",
            format!("# v2 git bundle\n{A} refs/heads/x\r\n\nPACK").into_bytes(),
            BundleRefusal::MalformedLine(2),
        ),
        (
            "a ref line without a name",
            bundle(&["# v2 git bundle", A]),
            BundleRefusal::MalformedLine(2),
        ),
        (
            "hash one too long",
            bundle(&["# v2 git bundle", &format!("{long_hash} refs/heads/x")]),
            BundleRefusal::BadHash(long_hash.clone()),
        ),
        (
            "hash one too short",
            bundle(&["# v2 git bundle", &format!("-{}", &A[..39])]),
            BundleRefusal::BadHash(A[..39].to_owned()),
        ),
        (
            "uppercase hash",
            bundle(&[
                "# v2 git bundle",
                &format!("{} refs/heads/x", A.to_uppercase()),
            ]),
            BundleRefusal::BadHash(A.to_uppercase()),
        ),
        (
            "ref name with ..",
            bundle(&["# v2 git bundle", &format!("{A} refs/heads/a..b")]),
            BundleRefusal::BadRefName("refs/heads/a..b".to_owned()),
        ),
        (
            "no end of header",
            format!("# v2 git bundle\n{A} refs/heads/x\n").into_bytes(),
            BundleRefusal::NoEndOfHeader,
        ),
    ];
    for (name, bytes, refusal) in cases {
        assert_eq!(parse_header(&bytes), Err(refusal), "case {name}");
    }
}

#[test]
fn a_header_longer_than_the_cap_is_refused() {
    let mut bytes = b"# v2 git bundle\n".to_vec();
    let line = format!("-{A} padding padding padding\n");
    while bytes.len() <= HEADER_CAP {
        bytes.extend_from_slice(line.as_bytes());
    }
    assert_eq!(parse_header(&bytes), Err(BundleRefusal::HeaderTooLong));
    // Ending it after the cap changes nothing: the end is never looked for past it.
    bytes.extend_from_slice(b"\nPACK");
    assert_eq!(parse_header(&bytes), Err(BundleRefusal::HeaderTooLong));
}

#[test]
fn every_refusal_says_what_it_refused() {
    for refusal in [
        BundleRefusal::NotABundle,
        BundleRefusal::Sha256,
        BundleRefusal::Filter("blob:none".to_owned()),
        BundleRefusal::HeaderTooLong,
    ] {
        let sentence = refusal.to_string();
        assert!(sentence.contains("bundle"), "{sentence}");
    }
    assert!(
        BundleRefusal::BadHash("xyz".to_owned())
            .to_string()
            .contains("xyz")
    );
    assert!(BundleRefusal::Sha256.to_string().contains("sha256"));
}

/// Runs `git` in `dir` with the same location variables removed as the crate's own verbs do, and
/// returns its trimmed stdout; panics on a non-zero exit.
fn git(cli: &Cli, dir: &Path, args: &[&str]) -> String {
    let mut command = std::process::Command::new(cli.binary());
    command.current_dir(dir).args(args).env("LC_ALL", "C");
    for key in SCRUBBED_ENV {
        command.env_remove(key);
    }
    let output = command.output().expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn dir_in(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir(&dir).expect("the directory is created");
    dir
}

fn offset_of(path: &Path) -> u64 {
    read_header(path).expect("the header parses").pack_offset
}

#[tokio::test]
async fn round_trips_a_real_bundle_through_index_pack() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let root = tempfile::tempdir().expect("a temporary directory");
    let host = dir_in(root.path(), "host");
    let base = repo_with_one_commit(&host);
    let tip = commit_file(&host, "g", "two\n", "two");
    create_ref(&host, "refs/heads/seed", &base).expect("seed ref");
    create_ref(&host, "refs/heads/run", &tip).expect("run ref");

    let seed = root.path().join("seed.bundle");
    cli.bundle_create(&host, "refs/heads/seed", &seed)
        .await
        .expect("bundle create");
    let header = read_header(&seed).expect("git's own header parses");
    assert!(header.prerequisites.is_empty());
    assert_eq!(
        header.refs,
        vec![(base.clone(), "refs/heads/seed".to_owned())]
    );
    let bytes = std::fs::read(&seed).expect("the bundle reads");
    let offset = usize::try_from(header.pack_offset).expect("small");
    assert_eq!(&bytes[offset..offset + 4], b"PACK");
    cli.bundle_verify(&host, &seed)
        .await
        .expect("bundle verify");

    // An existing output is never overwritten.
    let again = cli.bundle_create(&host, "refs/heads/run", &seed).await;
    assert!(matches!(again, Err(IsolateError::Refused(_))), "{again:?}");
    assert_eq!(std::fs::read(&seed).expect("still there"), bytes);

    let increment = root.path().join("increment.bundle");
    git(
        &cli,
        &host,
        &[
            "bundle",
            "create",
            increment.to_str().expect("utf-8"),
            "refs/heads/run",
            &format!("^{base}"),
        ],
    );
    let header = read_header(&increment).expect("an incremental header parses");
    assert_eq!(header.prerequisites, vec![base.clone()]);
    assert_eq!(
        header.refs,
        vec![(tip.clone(), "refs/heads/run".to_owned())]
    );

    let target = dir_in(root.path(), "target");
    empty_repo(&target);
    assert!(
        cli.bundle_verify(&target, &increment).await.is_err(),
        "the base is missing"
    );
    cli.index_pack(&target, &seed, offset_of(&seed), CAP)
        .await
        .expect("the seed pack indexes");
    create_ref(&target, "refs/heads/run", &base).expect("the run branch");
    assert!(has_commit(&target, &base).expect("lookup"));
    assert!(!has_commit(&target, &tip).expect("lookup"));

    cli.bundle_verify(&target, &increment)
        .await
        .expect("verify with the base present");
    cli.index_pack(&target, &increment, header.pack_offset, CAP)
        .await
        .expect("the increment indexes");
    assert!(has_commit(&target, &tip).expect("lookup"));
    assert!(
        cli.connected(&target, &tip, std::slice::from_ref(&base))
            .await
            .expect("rev-list")
    );
    assert!(cli.connected(&target, &tip, &[]).await.expect("rev-list"));
}

#[tokio::test]
async fn strict_index_pack_refuses_a_commit_without_an_email_and_keeps_nothing() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let root = tempfile::tempdir().expect("a temporary directory");
    let host = dir_in(root.path(), "host");
    let base = repo_with_one_commit(&host);
    let tree = git(&cli, &host, &["rev-parse", &format!("{base}^{{tree}}")]);
    let text = root.path().join("bad.txt");
    std::fs::write(
        &text,
        format!(
            "tree {tree}\nparent {base}\nauthor Nobody 1600000000 +0000\n\
             committer Nobody <n@localhost> 1600000000 +0000\n\nno email\n"
        ),
    )
    .expect("the commit text is written");
    let bad = git(
        &cli,
        &host,
        &[
            "hash-object",
            "--literally",
            "-t",
            "commit",
            "-w",
            text.to_str().expect("utf-8"),
        ],
    );
    create_ref(&host, "refs/heads/seed", &base).expect("seed ref");
    create_ref(&host, "refs/heads/bad", &bad).expect("bad ref");
    let seed = root.path().join("seed.bundle");
    cli.bundle_create(&host, "refs/heads/seed", &seed)
        .await
        .expect("bundle create");
    let hostile = root.path().join("bad.bundle");
    git(
        &cli,
        &host,
        &[
            "bundle",
            "create",
            hostile.to_str().expect("utf-8"),
            "refs/heads/bad",
            &format!("^{base}"),
        ],
    );

    let strict = dir_in(root.path(), "strict");
    let lax = dir_in(root.path(), "lax");
    for target in [&strict, &lax] {
        empty_repo(target);
        cli.index_pack(target, &seed, offset_of(&seed), CAP)
            .await
            .expect("the seed pack indexes");
    }

    let refused = cli
        .index_pack(&strict, &hostile, offset_of(&hostile), CAP)
        .await;
    assert!(
        matches!(&refused, Err(IsolateError::Refused(sentence)) if sentence.contains("fsck")),
        "{refused:?}"
    );
    assert!(
        !has_commit(&strict, &bad).expect("lookup"),
        "nothing was kept"
    );

    // The same pack without `--strict` is taken, which is what makes the flag the check: `git`
    // itself, since `index_pack` is always strict (R1b L5).
    let pack = std::fs::read(&hostile).expect("the bundle reads");
    let offset = usize::try_from(offset_of(&hostile)).expect("small");
    let mut lax_pack = std::process::Command::new(cli.binary());
    lax_pack
        .current_dir(&lax)
        .args(["index-pack", "--stdin", "--fix-thin"]);
    for key in SCRUBBED_ENV {
        lax_pack.env_remove(key);
    }
    let mut lax_pack = lax_pack
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("git runs");
    {
        use std::io::Write as _;
        let mut stdin = lax_pack.stdin.take().expect("a pipe");
        stdin
            .write_all(&pack[offset..])
            .expect("the pack is written");
    }
    assert!(
        lax_pack.wait().expect("git exits").success(),
        "a lax index-pack takes it"
    );
    assert!(has_commit(&lax, &bad).expect("lookup"));
}

/// R1b M4: `--max-input-size` refuses a pack over the cap and keeps nothing.
#[tokio::test]
async fn index_pack_refuses_a_pack_over_its_cap() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let root = tempfile::tempdir().expect("a temporary directory");
    let host = dir_in(root.path(), "host");
    let base = repo_with_one_commit(&host);
    create_ref(&host, "refs/heads/seed", &base).expect("seed ref");
    let seed = root.path().join("seed.bundle");
    cli.bundle_create(&host, "refs/heads/seed", &seed)
        .await
        .expect("bundle create");
    let target = dir_in(root.path(), "target");
    empty_repo(&target);
    let refused = cli.index_pack(&target, &seed, offset_of(&seed), 16).await;
    assert!(
        matches!(&refused, Err(IsolateError::Refused(_))),
        "{refused:?}"
    );
    assert!(!has_commit(&target, &base).expect("lookup"));
}

/// R1b M1, M4: the object-kind and ancestry reads `collect` makes through a quarantine.
#[tokio::test]
async fn object_kind_and_ancestry_read_through_a_quarantine() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let root = tempfile::tempdir().expect("a temporary directory");
    let host = dir_in(root.path(), "host");
    let base = repo_with_one_commit(&host);
    let tip = commit_file(&host, "g", "two\n", "two");
    create_ref(&host, "refs/heads/seed", &base).expect("seed ref");
    create_ref(&host, "refs/heads/run", &tip).expect("run ref");
    let increment = root.path().join("increment.bundle");
    git(
        &cli,
        &host,
        &[
            "bundle",
            "create",
            increment.to_str().expect("utf-8"),
            "refs/heads/run",
            &format!("^{base}"),
        ],
    );
    let seed = root.path().join("seed.bundle");
    cli.bundle_create(&host, "refs/heads/seed", &seed)
        .await
        .expect("bundle create");
    let target = dir_in(root.path(), "target");
    empty_repo(&target);
    cli.index_pack(&target, &seed, offset_of(&seed), CAP)
        .await
        .expect("the seed pack indexes");

    let quarantine = ObjectQuarantine {
        dir: dir_in(root.path(), "quarantine"),
        objects: objects_dir(&target).expect("the objects dir"),
    };
    let held = cli.in_quarantine(&quarantine);
    held.index_pack(&target, &increment, offset_of(&increment), CAP)
        .await
        .expect("the increment indexes into the quarantine");
    assert!(
        !has_commit(&target, &tip).expect("lookup"),
        "not in the repository"
    );
    assert_eq!(
        cli.object_kind(&target, &tip).await.expect("cat-file"),
        None
    );
    assert_eq!(
        held.object_kind(&target, &tip)
            .await
            .expect("cat-file")
            .as_deref(),
        Some("commit")
    );
    assert!(
        held.connected(&target, &tip, std::slice::from_ref(&base))
            .await
            .expect("rev-list")
    );
    assert!(
        held.is_ancestor_of(&target, &base, &tip)
            .await
            .expect("merge-base")
    );
    assert!(
        !held
            .is_ancestor_of(&target, &tip, &base)
            .await
            .expect("merge-base")
    );
    assert!(
        cli.is_ancestor_of(&target, &base, &tip).await.is_err(),
        "the tip is not here"
    );
    assert!(cli.object_kind(&target, "--all").await.is_err());

    migrate_quarantine(&quarantine).expect("the move");
    assert!(
        has_commit(&target, &tip).expect("lookup"),
        "moved into the repository"
    );
    assert!(cli.connected(&target, &tip, &[]).await.expect("rev-list"));
}

#[tokio::test]
async fn a_tip_whose_parent_is_missing_is_not_connected() {
    let Some(cli) = skip_without_git!() else {
        return;
    };
    let root = tempfile::tempdir().expect("a temporary directory");
    let repo = dir_in(root.path(), "repo");
    let base = repo_with_one_commit(&repo);
    let tree = git(&cli, &repo, &["rev-parse", &format!("{base}^{{tree}}")]);
    let text = root.path().join("orphan.txt");
    std::fs::write(
        &text,
        format!(
            "tree {tree}\nparent {}\nauthor A <a@localhost> 1600000000 +0000\n\
             committer A <a@localhost> 1600000000 +0000\n\nparent is missing\n",
            "1".repeat(40)
        ),
    )
    .expect("the commit text is written");
    let orphan = git(
        &cli,
        &repo,
        &[
            "hash-object",
            "--literally",
            "-t",
            "commit",
            "-w",
            text.to_str().expect("utf-8"),
        ],
    );
    assert!(has_commit(&repo, &orphan).expect("lookup"));
    assert!(cli.connected(&repo, &base, &[]).await.expect("rev-list"));
    assert!(!cli.connected(&repo, &orphan, &[]).await.expect("rev-list"));
    assert!(
        !cli.connected(&repo, &orphan, std::slice::from_ref(&base))
            .await
            .expect("rev-list")
    );
    assert!(
        cli.connected(&repo, "--all", &[]).await.is_err(),
        "an option is not a hash"
    );
    assert!(
        cli.connected(&repo, &base, &["--all".to_owned()])
            .await
            .is_err()
    );
}

#[test]
fn create_ref_refuses_an_existing_ref_and_delete_ref_reports_absence() {
    let root = tempfile::tempdir().expect("a temporary directory");
    let repo = root.path().join("repo");
    std::fs::create_dir(&repo).expect("dir");
    let base = repo_with_one_commit(&repo);
    let tip = commit_file(&repo, "g", "two\n", "two");
    let temp = "refs/htui-collect/00000000-0000-0000-0000-000000000001/00000000-0000-0000-0000-000000000002";

    assert_eq!(ref_target(&repo, temp).expect("read"), None);
    assert!(!delete_ref(&repo, temp).expect("delete of an absent ref"));
    create_ref(&repo, temp, &base).expect("create");
    assert_eq!(ref_target(&repo, temp).expect("read"), Some(base.clone()));
    assert!(create_ref(&repo, temp, &tip).is_err(), "MustNotExist");
    assert_eq!(ref_target(&repo, temp).expect("read"), Some(base));
    assert!(delete_ref(&repo, temp).expect("delete"));
    assert_eq!(ref_target(&repo, temp).expect("read"), None);
    assert!(!delete_ref(&repo, temp).expect("second delete"));
}

//! `keys.toml` end to end (MOD-67 M2; ANA-26 §6.3, §7.4, §7.5): the fixtures under
//! `tests/fixtures/keys/` through the real binary and through the shell.
//!
//! - `binary` (Unix): `htui --keys F --print-keys` prints the table (exit 0) or the exact §7.5
//!   report on stderr (exit 2, nothing on stdout); a bad file stops the TUI before the terminal;
//!   a missing named file exits 2; clap refuses `--keys` beside `--default-keys`. An entry that
//!   takes `ctrl-q` from the defaulted queue loads with a notice on stderr (MOD-12 M3 R1 H1), and
//!   so does a global one that takes `ctrl-x` from the defaulted editor abort (MOD-57 M2 R1 M-1).
//! - `default_path` (Linux): the config directory's `keys.toml` is read, a missing one is the
//!   defaults and creates nothing, `--default-keys` ignores a broken one.
//! - `app` (`testkit`): a shell whose `app.keys` came from `valid.toml` quits on the rebound chord
//!   and no longer on `q`, closes an overlay on `F2`, and shows both on the status line and the
//!   `?` box; one from `quit_ctrl_q.toml` quits on `ctrl-q` and never opens the queue.
//! - `reference_page` (MOD-67 M6 D8): `docs/htui-keys.md`'s toml block is the default print.
//!
//! No case may read the developer's own `keys.toml` (blueprint F-9, H-10): every child runs with
//! a cleared environment, `HOME` and `XDG_CONFIG_HOME` both set to a temporary directory, and
//! passes `--keys` or `--default-keys` unless it looks up the default path, which only Linux
//! resolves through `XDG_CONFIG_HOME`. Every child ends in 0 or a refusal (2), so nothing reaches
//! Sentry (H-9).

/// `tests/fixtures/keys/<name>.toml`. Only the Unix binary cases and the `testkit` App case read
/// fixtures, so it is compiled for them alone (review L2).
#[cfg(any(unix, feature = "testkit"))]
fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/keys")
        .join(format!("{name}.toml"))
}

/// The report's last line (ANA-26 §7.5), as `main` prints it.
#[cfg(unix)]
const HINT: &str = "Fix the file, or run `htui --default-keys` to start with the default keys.\n";

/// The spawned binary, isolated from the developer's configuration.
#[cfg(unix)]
mod child {
    use std::path::Path;
    use std::process::{Command, Output, Stdio};

    const HTUI: &str = env!("CARGO_BIN_EXE_htui");

    /// Runs `htui args…` with only `PATH` from this process, and `HOME` and `XDG_CONFIG_HOME`
    /// both `home`, stdin closed.
    pub(crate) fn run(args: &[&str], home: &Path) -> Output {
        let mut command = Command::new(HTUI);
        command
            .args(args)
            .env_clear()
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home)
            .stdin(Stdio::null());
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        command.output().expect("htui starts")
    }

    /// The child's stdout and stderr as text.
    pub(crate) fn text(output: &Output) -> (String, String) {
        (
            String::from_utf8(output.stdout.clone()).expect("stdout is UTF-8"),
            String::from_utf8(output.stderr.clone()).expect("stderr is UTF-8"),
        )
    }
}

#[cfg(unix)]
mod binary {
    use super::child::{run, text};
    use super::{HINT, fixture};
    use htui::keys::{Keys, load_path, print};

    #[test]
    fn print_keys_prints_a_valid_file() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let valid = fixture("valid");
        let output = run(
            &["--keys", valid.to_str().expect("UTF-8"), "--print-keys"],
            home.path(),
        );
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert_eq!(stderr, "");
        assert_eq!(stdout, print(&load_path(&valid).expect("valid.toml loads")));
        // MOD-115 M2: a version-1 quit keeps `ctrl-c` (D-9); `Esc` is never printed (MD7).
        assert!(
            stdout.contains("quit         = [\"ctrl-y\", \"ctrl-c\"]  # quit (changed)\n"),
            "{stdout}"
        );
        assert!(
            stdout.contains("close = [\"f2\"]  # close; Esc always (changed)\n"),
            "{stdout}"
        );
    }

    /// MOD-115 M2 (MD2, MD7) end to end: `ctrl-c` binds like any chord, and a file that empties
    /// a row holding `Esc` loads with `Esc` kept, each printing what it loaded.
    #[test]
    fn ctrl_c_binds_and_an_emptied_esc_row_keeps_esc() {
        let home = tempfile::tempdir().expect("a throwaway home");
        for (name, line) in [
            (
                "ctrl_c",
                "find         = [\"ctrl-c\"]      # find (changed)\n",
            ),
            ("overlay_close", "close = []  # close; Esc always\n"),
        ] {
            let path = fixture(name);
            let output = run(
                &["--keys", path.to_str().expect("UTF-8"), "--print-keys"],
                home.path(),
            );
            let (stdout, stderr) = text(&output);
            assert_eq!(output.status.code(), Some(0), "{name}: {stderr}");
            assert_eq!(stderr, "", "{name}");
            assert_eq!(
                stdout,
                print(&load_path(&path).expect("the file loads")),
                "{name}"
            );
            assert!(stdout.contains(line), "{name}: {stdout}");
        }
    }

    #[test]
    fn print_keys_with_default_keys_prints_the_defaults() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let output = run(&["--default-keys", "--print-keys"], home.path());
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert_eq!(stderr, "");
        assert_eq!(stdout, print(Keys::compiled()));
        assert!(!stdout.contains("(changed)"), "{stdout}");
    }

    /// Every bad fixture with the lines of its report (blueprint §11.2).
    fn bad_fixtures() -> [(&'static str, Vec<String>); 5] {
        [
            (
                "errors",
                vec![
                    "2: version: write version = 2 (version = 1 is a file from before MOD-115)"
                        .to_owned(),
                    r#"5: [list] down: write a chord such as "q", or a list such as ["q", "x"]"#
                        .to_owned(),
                    r#"6: [list] up: write each chord as a string, such as "q""#.to_owned(),
                    "8: [globl]: no such table; the tables are global, overlay, list, pane, \
                     confirm, form, common, editor, settings, settings.agents, settings.hierarchy, \
                     settings.kinds, settings.prompt, settings.connection, settings.qdrant, \
                     settings.boxes, settings.personas, settings.secrets, settings.queue, \
                     concepts, switcher, migration, waiting, skills, skills.library, \
                     skills.templates, skills.attach, skills.help, requirements, backlog, \
                     backlog.body, backlog.runs, backlog.graph, backlog.docs, backlog.notes, \
                     backlog.prompt, backlog.reqs, backlog.filter_form, backlog.item, chat, \
                     queue, picker, layout, history, leave, field, choice, answer"
                        .to_owned(),
                    "12: [global] quitt: no such action; [global] has quit, next_tab, prev_tab, \
                     select_tab_1, select_tab_2, select_tab_3, select_tab_4, select_tab_5, \
                     select_tab_6, select_tab_7, select_tab_8, select_tab_9, help, about, workspaces, \
                     find, waiting, queue, next_pane, prev_pane"
                        .to_owned(),
                    r#"13: [global] quit = "shift-a": write a shifted letter as "A""#.to_owned(),
                    r#"13: [global] quit = "x": "x" is already settings.agents.cancel (default) in Settings > Agents"#
                        .to_owned(),
                    r#"13: [global] quit = "x": "x" is already skills.attach.detach (default) in Skills > Library > Attachments"#
                        .to_owned(),
                    r#"13: [global] quit = "x": "x" is already backlog.runs.reject (default) in Backlog > Runs"#
                        .to_owned(),
                ],
            ),
            (
                "esc",
                vec![
                    r#"3: [global] quit = "esc": esc always closes, cancels or goes back and cannot be bound"#
                        .to_owned(),
                    r#"4: [global] find = "Escape": esc always closes, cancels or goes back and cannot be bound"#
                        .to_owned(),
                ],
            ),
            (
                "quit_unbound",
                vec![
                    "5: [global] quit: must keep at least one chord: global.quit opens the leave \
                     dialog and leave.confirm leaves; without both htui cannot be left"
                        .to_owned(),
                ],
            ),
            (
                "collision",
                vec![
                    r#"5: [global] quit = "w": "w" is already global.workspaces (line 4) in [global]"#
                        .to_owned(),
                    r#"5: [global] quit = "w": "w" is already settings.boxes.executor (default) in Settings > Boxes"#
                        .to_owned(),
                    // MOD-115 M2 MD7: `esc` itself is refused, before any collision.
                    r#"6: [global] help = "esc": esc always closes, cancels or goes back and cannot be bound"#
                        .to_owned(),
                    r#"9: [form] save = "s": "s" is typed text while a field captures: bind a ctrl or alt chord or a named key"#
                        .to_owned(),
                    r#"13: [list] top = "j": "j" is already list.down (line 12) in [list]"#
                        .to_owned(),
                ],
            ),
            ("syntax", vec!["4: not valid TOML: duplicate key".to_owned()]),
        ]
    }

    /// The whole stderr `main` prints for `name`'s fixture at `path`.
    fn report(name: &str, path: &std::path::Path, lines: &[String]) -> String {
        let count = lines.len();
        let plural = if count == 1 { "" } else { "s" };
        let mut expected = format!("htui: {} has {count} error{plural}:\n", path.display());
        for line in lines {
            expected.push_str(&format!("  {name}.toml:{line}\n"));
        }
        expected.push_str(HINT);
        expected
    }

    #[test]
    fn each_bad_fixture_exits_2_with_its_exact_report_and_nothing_on_stdout() {
        let home = tempfile::tempdir().expect("a throwaway home");
        for (name, lines) in bad_fixtures() {
            let path = fixture(name);
            let output = run(
                &["--keys", path.to_str().expect("UTF-8"), "--print-keys"],
                home.path(),
            );
            let (stdout, stderr) = text(&output);
            assert_eq!(output.status.code(), Some(2), "{name}: {stderr}");
            assert_eq!(stdout, "", "{name}");
            assert_eq!(stderr, report(name, &path, &lines), "{name}");
        }
    }

    /// D5, H-12: the refusal precedes `connect::start` and `terminal::init`. The child has no
    /// TTY: a late load point would fail differently, or start a backend first.
    #[test]
    fn a_bad_file_also_stops_the_tui_before_the_terminal() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let path = fixture("errors");
        let output = run(
            &["--keys", path.to_str().expect("UTF-8"), "--offline"],
            home.path(),
        );
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert_eq!(stdout, "");
        let (_, lines) = bad_fixtures()
            .into_iter()
            .find(|(name, _)| *name == "errors")
            .expect("errors is a bad fixture");
        assert_eq!(stderr, report("errors", &path, &lines));
        assert!(
            std::fs::read_dir(home.path())
                .expect("the home is readable")
                .next()
                .is_none(),
            "nothing was created under the config root"
        );
    }

    #[test]
    fn a_missing_named_file_exits_2() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let path = home.path().join("missing.toml");
        let output = run(
            &["--keys", path.to_str().expect("UTF-8"), "--print-keys"],
            home.path(),
        );
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert_eq!(stdout, "");
        let head = format!("htui: cannot read {}: ", path.display());
        assert!(stderr.starts_with(&head), "{stderr}");
        assert!(stderr.ends_with(&format!("\n{HINT}")), "{stderr}");
    }

    /// MOD-12 M3 D8 made `ctrl-q` `global.queue`'s default. MOD-12 M3 R1 H1: a file written
    /// before it that binds `ctrl-q` in `[global]` still loads. The user's entry wins, the queue
    /// (which the file leaves at its default) is unbound, stderr says so, and `--print-keys`
    /// marks the queue row with the action that took its chord.
    #[test]
    fn a_global_ctrl_q_binding_takes_ctrl_q_from_the_queue_with_a_notice() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let path = fixture("quit_ctrl_q");
        let output = run(
            &["--keys", path.to_str().expect("UTF-8"), "--print-keys"],
            home.path(),
        );
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert_eq!(
            stderr,
            format!(
                "htui: {}:4: [global] quit = \"ctrl-q\" takes \"ctrl-q\" from global.queue, which \
                 is now unbound\n",
                path.display()
            )
        );
        assert_eq!(stdout, print(&load_path(&path).expect("the file loads")));
        let row = |name: &str| {
            stdout
                .lines()
                .find(|line| line.starts_with(&format!("{name} ")))
                .unwrap_or_else(|| panic!("no {name} row in {stdout}"))
                .to_owned()
        };
        assert!(row("queue").contains("= []"), "{stdout}");
        assert!(
            row("queue").ends_with("# queue (unbound by quit)"),
            "{stdout}"
        );
        // MOD-115 M2 (D-9): a version-1 quit keeps `ctrl-c`.
        assert!(
            row("quit").contains(r#"= ["ctrl-q", "ctrl-c"]"#),
            "{stdout}"
        );
        assert!(row("quit").ends_with("# quit (changed)"), "{stdout}");
    }

    /// MOD-57 M2 R1 M-1: the README's example before M2 binds `ctrl-x` in `[global]`, which
    /// `editor.abort` holds by default while an edit is open. The file still loads: the global
    /// entry takes `ctrl-x`, stderr says so, and `--print-keys` marks the abort row with it.
    #[test]
    fn a_global_ctrl_x_binding_takes_ctrl_x_from_the_editor_abort_with_a_notice() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let path = fixture("quit_ctrl_x");
        let output = run(
            &["--keys", path.to_str().expect("UTF-8"), "--print-keys"],
            home.path(),
        );
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert_eq!(
            stderr,
            format!(
                "htui: {}:7: [global] quit = \"ctrl-x\" takes \"ctrl-x\" from editor.abort, which \
                 is now unbound\n",
                path.display()
            )
        );
        assert_eq!(stdout, print(&load_path(&path).expect("the file loads")));
        let row = |name: &str| {
            stdout
                .lines()
                .find(|line| line.starts_with(&format!("{name} ")))
                .unwrap_or_else(|| panic!("no {name} row in {stdout}"))
                .to_owned()
        };
        assert!(row("abort").contains("= []"), "{stdout}");
        assert!(
            row("abort").ends_with("# abort edit (unbound by global.quit)"),
            "{stdout}"
        );
        assert!(row("quit").ends_with("# quit (changed)"), "{stdout}");
    }

    /// Two entries of the file on one chord are still refused (MOD-67 M2 D8 step 5), even when
    /// one of them repeats its action's default.
    #[test]
    fn two_entries_on_ctrl_q_are_still_refused() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let path = home.path().join("both.toml");
        std::fs::write(
            &path,
            "[global]\nquit = [\"ctrl-q\"]\nqueue = [\"ctrl-q\"]\n",
        )
        .expect("the file is written");
        let output = run(
            &["--keys", path.to_str().expect("UTF-8"), "--print-keys"],
            home.path(),
        );
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert_eq!(stdout, "");
        let line =
            r#"2: [global] quit = "ctrl-q": "ctrl-q" is already global.queue (line 3) in [global]"#;
        assert_eq!(stderr, report("both", &path, &[line.to_owned()]));
    }

    /// MOD-115 M2 D7 end to end: the filter form's option keys are `[choice]` rows, so a
    /// version-2 file that gives one to another of the form's acts is an ordinary collision,
    /// refused before anything starts on the entry's line, and one that also moves the choice
    /// row off it loads.
    #[test]
    fn a_rebind_onto_a_choice_key_collides_and_moving_the_choice_frees_it() {
        let home = tempfile::tempdir().expect("a throwaway home");
        for (name, src, line) in [
            (
                "space",
                "version = 2\n[backlog.filter_form]\nclear = [\"space\"]\n",
                Some(
                    r#"3: [backlog.filter_form] clear = "space": "space" is already choice.toggle (default) in the Backlog filter form"#,
                ),
            ),
            (
                "moved",
                "version = 2\n[choice]\ntoggle = [\"t\"]\n[backlog.filter_form]\nclear = [\"space\"]\n",
                None,
            ),
        ] {
            let path = home.path().join(format!("{name}.toml"));
            std::fs::write(&path, src).expect("the file is written");
            let output = run(
                &["--keys", path.to_str().expect("UTF-8"), "--print-keys"],
                home.path(),
            );
            let (stdout, stderr) = text(&output);
            match line {
                Some(line) => {
                    assert_eq!(output.status.code(), Some(2), "{name}: {stderr}");
                    assert_eq!(stdout, "", "{name}");
                    assert_eq!(stderr, report(name, &path, &[line.to_owned()]), "{name}");
                }
                None => {
                    assert_eq!(output.status.code(), Some(0), "{name}: {stderr}");
                    assert!(stdout.contains("toggle = [\"t\"]"), "{name}: {stdout}");
                }
            }
        }
    }

    /// MOD-67 M4 D8.1 end to end: a file that leaves the in-pane editor no way out is refused
    /// before anything starts, on the entry's line.
    #[test]
    fn an_editor_file_that_unbinds_focus_exits_2() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let path = home.path().join("editor.toml");
        std::fs::write(&path, "[editor]\nfocus = []\n").expect("the file is written");
        let output = run(
            &["--keys", path.to_str().expect("UTF-8"), "--print-keys"],
            home.path(),
        );
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert_eq!(stdout, "");
        let line = "2: [editor] focus: must keep at least one chord: the editor takes every \
                    other key";
        assert_eq!(stderr, report("editor", &path, &[line.to_owned()]));
    }

    #[test]
    fn keys_and_default_keys_are_refused_together() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let valid = fixture("valid");
        let output = run(
            &["--keys", valid.to_str().expect("UTF-8"), "--default-keys"],
            home.path(),
        );
        let (_, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert!(stderr.contains("cannot be used with"), "{stderr}");
    }
}

/// The file in the config directory: only Linux reads `XDG_CONFIG_HOME` (F-9).
#[cfg(target_os = "linux")]
mod default_path {
    use super::HINT;
    use super::child::{run, text};
    use htui::keys::{Keys, print};

    /// A key file with one error on line 2 (MOD-115 M2 MD7: `Esc` is never listed).
    const BROKEN: &str = "[global]\nquit = \"esc\"\n";

    /// `<home>/htui/keys.toml` holding `BROKEN`.
    fn broken_file(home: &std::path::Path) -> std::path::PathBuf {
        let dir = home.join("htui");
        std::fs::create_dir(&dir).expect("the config directory");
        let path = dir.join("keys.toml");
        std::fs::write(&path, BROKEN).expect("the key file");
        path
    }

    #[test]
    fn the_config_directorys_file_is_read() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let path = broken_file(home.path());
        let output = run(&["--print-keys"], home.path());
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(2), "{stderr}");
        assert_eq!(stdout, "");
        assert_eq!(
            stderr,
            format!(
                "htui: {} has 1 error:\n  keys.toml:2: [global] quit = \"esc\": esc always \
                 closes, cancels or goes back and cannot be bound\n{HINT}",
                path.display()
            )
        );
    }

    /// D3 and T1's non-creating `config_root_path`, end to end.
    #[test]
    fn no_file_is_the_defaults_and_creates_nothing() {
        let home = tempfile::tempdir().expect("a throwaway home");
        let output = run(&["--print-keys"], home.path());
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert_eq!(stdout, print(Keys::compiled()));
        assert!(
            !home.path().join("htui").exists(),
            "`--print-keys` created the config directory"
        );
    }

    #[test]
    fn default_keys_ignores_a_broken_file() {
        let home = tempfile::tempdir().expect("a throwaway home");
        broken_file(home.path());
        let output = run(&["--default-keys", "--print-keys"], home.path());
        let (stdout, stderr) = text(&output);
        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert_eq!(stdout, print(Keys::compiled()));
    }
}

/// The shell over `valid.toml`'s keys, set before `register_all` as `lib::run` does (D5).
#[cfg(feature = "testkit")]
mod app {
    use super::fixture;
    use htui::agent_worker::AgentRuntime;
    use htui::app::register_all;
    use htui::testkit::Harness;
    use htui::ui::overlay::WorkspaceSwitcher;
    use htui_agent::registry::DriverFactory;

    /// The keys line the status row listed before MOD-115 (`tests/keys.rs`'s).
    fn status_line(harness: &mut Harness) -> String {
        harness.app().keys_line()
    }

    #[tokio::test]
    async fn a_rebound_quit_and_close_work_end_to_end() {
        let keys = htui::keys::load_path(&fixture("valid")).expect("valid.toml loads");
        let mut harness =
            Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
        harness.app().keys = keys;
        register_all(harness.app());
        harness.drive_to_end().await;

        harness.key("q");
        assert!(!harness.app().should_quit, "`q` no longer quits");
        let line = status_line(&mut harness);
        assert!(line.starts_with("Ctrl+y quit · Tab next tab"), "{line}");

        harness.key("?");
        assert!(harness.app().help_visible, "`?` opens the box");
        let frame = harness.render();
        assert!(frame.contains("Ctrl+y/Ctrl+c quit"), "{frame}");
        harness.key("?");
        assert!(!harness.app().help_visible, "`?` closes it");

        harness.key("w");
        harness.drive_to_end().await;
        assert_eq!(
            harness.app().overlays.top().map(|overlay| overlay.id()),
            Some(WorkspaceSwitcher::ID)
        );
        harness.key("f2");
        assert!(
            harness.app().overlays.is_empty(),
            "`F2` closes the switcher"
        );

        harness.key("w");
        harness.drive_to_end().await;
        harness.key("?");
        let frame = harness.render();
        assert!(frame.contains("Overlay: Esc/F2 close"), "{frame}");
        harness.key("?");
        harness.key("esc");
        assert!(harness.app().overlays.is_empty(), "`Esc` still closes it");

        harness.key("ctrl-y");
        assert!(harness.app().should_quit, "`Ctrl+y` quits");
    }

    /// MOD-12 M3 R1 H1: the shell over a file that gives `ctrl-q` to quit starts, shows quit on
    /// it, offers no queue key, and quits on it without opening the queue.
    #[tokio::test]
    async fn a_quit_on_ctrl_q_quits_and_leaves_the_queue_unbound() {
        let keys = htui::keys::load_path(&fixture("quit_ctrl_q")).expect("the file loads");
        let mut harness =
            Harness::demo().with_agent_runtime(AgentRuntime::new(DriverFactory::new()));
        harness.app().keys = keys;
        register_all(harness.app());
        harness.drive_to_end().await;

        let line = status_line(&mut harness);
        assert!(line.starts_with("Ctrl+q quit · Tab next tab"), "{line}");
        harness.key("?");
        let frame = harness.render();
        assert!(frame.contains("Ctrl+q/Ctrl+c quit"), "{frame}");
        assert!(!frame.contains("Ctrl+q queue"), "{frame}");
        harness.key("?");
        assert!(!harness.app().help_visible, "`?` closes the box");

        harness.key("ctrl-q");
        assert!(
            harness.app().overlays.is_empty(),
            "`Ctrl+q` no longer opens the queue"
        );
        assert!(harness.app().should_quit, "`Ctrl+q` quits");
    }
}

/// `docs/htui-keys.md`, the generated key reference (MOD-67 M6 D8). No binary and no `testkit`:
/// the page is checked against the library's print.
mod reference_page {
    /// MOD-67 M6 D8: `docs/htui-keys.md`'s toml block is the default print, byte for byte.
    #[test]
    fn the_key_reference_page_is_the_default_print() {
        let page = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/htui-keys.md"
        ))
        .expect("docs/htui-keys.md exists");
        let block = page
            .split_once("```toml\n")
            .and_then(|(_, rest)| rest.split_once("```\n"))
            .map(|(block, _)| block)
            .expect("one ```toml block");
        assert!(
            block == htui::keys::print(&htui::keys::Keys::defaults()),
            "docs/htui-keys.md is stale: regenerate its toml block with \
             `cargo run -q -p htui -- --default-keys --print-keys`"
        );
    }
}

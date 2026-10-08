//! MOD-57 M1, T7: the in-pane editor end to end, with a real child on a real pseudo-terminal.
//!
//! The shell is the Templates harness (as `tests/templates.rs`), and the editor is a scripted
//! `sh` (the sandbox has no vi). Each case does what the event loop does with
//! `HTUI_EDITOR_PANE` set: `E` asks for the editor, `take_external_edit` hands the edit out,
//! `App::open_editor` spawns it through [`PtyChild::spawn`] with a sender of the case's own
//! channel, and [`pump`] stands in for the loop's pane arm and post-steps (the event, the draw,
//! `resize_editor`).
//!
//! Real time throughout: a paused clock beside a real child fires every timeout at once. Every
//! child belongs to the `App` the harness holds, so a failing assertion still kills it when the
//! harness drops.
#![cfg(all(unix, feature = "testkit"))]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use htui::app::register_all;
use htui::editor::pane::{PaneChild, PaneEvent, PtyChild};
use htui::editor::{EDITOR_ABORTED, EditorCommand};
use htui::testkit::Harness;
use htui_core::store::MemStore;
use tempfile::TempDir;
use tokio::sync::mpsc::{self, Receiver, Sender};

/// The event loop's bound on the pane's channel (`event_loop::PANE_QUEUE`, R1 M-1).
const QUEUE: usize = 256;

/// What a scripted editor prints once it is ready for keys.
const READY: &str = "pane-ready";

/// The demo world on the Skills tab's Templates view, every view registered.
async fn open() -> Harness {
    let mut harness = Harness::over(MemStore::demo());
    register_all(harness.app());
    harness.settle().await;
    harness.key("2");
    harness.key("l");
    harness.settle().await;
    harness
}

/// Moves the tree's cursor onto `name`: to the top, then down until the body pane is titled with
/// it.
fn select(harness: &mut Harness, name: &str) {
    for _ in 0..20 {
        harness.key("k");
    }
    let title = format!("\u{250c} {name} v");
    for _ in 0..20 {
        if harness.render().contains(&title) {
            return;
        }
        harness.key("j");
    }
    panic!("`{name}` is not in the tree:\n{}", harness.render());
}

/// The hint row: the last line of the tab's body, just above the shell's status line.
fn hint(frame: &str) -> String {
    let lines: Vec<&str> = frame.lines().collect();
    lines[lines.len() - 2].to_owned()
}

/// The notice: the rows between the content's bottom border and the hint, joined (it wraps).
fn notice(frame: &str) -> String {
    let lines: Vec<&str> = frame.lines().collect();
    let hint = lines.len() - 2;
    let bottom = lines[..hint]
        .iter()
        .rposition(|line| line.starts_with('\u{2514}'))
        .unwrap_or_else(|| panic!("no bottom border above the hint:\n{frame}"));
    lines[bottom + 1..hint]
        .iter()
        .map(|line| line.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `body` written to `editor.sh` in `dir` and resolved as `$VISUAL` the way the loop resolves it.
/// The value is `sh <script>`, so the script is read rather than executed: a fork elsewhere in
/// the test binary cannot make it `ETXTBSY`. `pty_command`'s `exec` makes that `sh` the child,
/// so `$$` in `body` is the editor's pid and `$1` the temp file.
fn script(dir: &TempDir, body: &str) -> EditorCommand {
    let path = dir.path().join("editor.sh");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write the script");
    let value = format!("sh {}", path.display());
    EditorCommand::resolve(|key| (key == "VISUAL").then(|| value.clone()))
}

/// The loop's pane step: takes the edit `E` asked for and opens it in the pane with `cmd`,
/// spawning through [`PtyChild::spawn`] with a clone of `events`. Returns the temp file (the
/// command's last argument).
fn open_editor(harness: &mut Harness, cmd: EditorCommand, events: &Sender<PaneEvent>) -> PathBuf {
    let Some((tab, edit)) = harness.app().take_external_edit() else {
        panic!("`E` asked for the editor");
    };
    let mut file = None;
    harness
        .app()
        .open_editor(tab, &edit, cmd, |id, command, size| {
            file = command.get_argv().last().map(PathBuf::from);
            PtyChild::spawn(command, size, id, events.clone())
                .map(|child| Box::new(child) as Box<dyn PaneChild>)
        });
    assert!(
        harness.app().editor_open(),
        "the editor opened: {:?}",
        harness.app().status
    );
    file.expect("the spawn was asked for")
}

/// The loop's pane arm and post-steps, until `until` holds (within 10 s): each event the pane
/// sends goes to `App::on_pane_event`, then a frame is drawn and `resize_editor` runs. Wakes every
/// 50 ms when nothing arrives, so `until` can wait on a file as well as on the screen.
async fn pump(
    harness: &mut Harness,
    rx: &mut Receiver<PaneEvent>,
    what: &str,
    mut until: impl FnMut(&mut Harness) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if until(harness) {
            return;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "no {what} within 10 s:\n{}",
            harness.render()
        );
        let wait = left.min(Duration::from_millis(50));
        if let Ok(Some(event)) = tokio::time::timeout(wait, rx.recv()).await {
            harness.app().on_pane_event(event);
        }
        harness.render();
        harness.app().resize_editor();
    }
}

/// The pid a script wrote to `path`, once it is there.
fn read_pid(path: &Path) -> Option<String> {
    let pid = std::fs::read_to_string(path).ok()?;
    let pid = pid.trim();
    (!pid.is_empty()).then(|| pid.to_owned())
}

/// As `editor::pane`'s real-child tests: a pid that `ps` no longer lists, or lists as a zombie, is
/// gone.
fn alive(pid: &str) -> bool {
    let out = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .expect("run ps");
    let stat = String::from_utf8_lossy(&out.stdout);
    let stat = stat.trim();
    !stat.is_empty() && !stat.starts_with('Z')
}

/// Waits (up to 3 s) for `pid` to be gone.
async fn gone(pid: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while alive(pid) {
        assert!(Instant::now() < deadline, "{pid} outlived the editor");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// PRD metric, plan T7: browse `E` in Templates opens the editor in the pane (over the whole tab
/// body: browse has no draft to claim, F-12), the keys typed reach it, and its exit reads the
/// file back into the view through the same outcome as the suspend mode.
#[tokio::test]
async fn e_in_templates_edits_in_the_pane_end_to_end() {
    let dir = TempDir::new().expect("a temp dir");
    let cmd = script(
        &dir,
        &format!("printf '{READY}\\n'; IFS= read -r line; printf '%s\\n' \"$line\" >> \"$1\""),
    );
    let mut harness = open().await;
    select(&mut harness, "implement");
    harness.key("E");
    let (tx, mut rx) = mpsc::channel(QUEUE);
    let file = open_editor(&mut harness, cmd, &tx);
    assert!(file.exists(), "the temp file is written before the spawn");

    pump(
        &mut harness,
        &mut rx,
        "ready editor with the keys",
        |harness| {
            let frame = harness.render();
            frame.contains(READY) && frame.contains("Ctrl+4 to htui")
        },
    )
    .await;
    for key in ["x", "y", "z", "enter"] {
        harness.key(key);
    }
    pump(&mut harness, &mut rx, "editor exit", |harness| {
        !harness.app().editor_open()
    })
    .await;
    harness.settle().await;
    // Before any key: `App::on_key` clears the status line.
    assert_eq!(harness.app().status, None, "the exit set no status");

    let frame = harness.render();
    assert!(
        notice(&frame).contains("edited in $EDITOR"),
        "the view heard `Edited`: {frame}"
    );
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "the view's editor is open on the draft: {frame}"
    );
    // The appended line, at the draft's end.
    harness.key("ctrl-end");
    let frame = harness.render();
    assert!(
        frame.contains("xyz"),
        "the typed line is in the draft: {frame}"
    );
    assert!(!frame.contains(READY), "the pane is gone: {frame}");
    assert!(!file.exists(), "the temp file is removed");
}

/// Plan T7: `Ctrl+E` on a draft hands it to the pane, which goes over the draft's claimed text
/// rect (inside the draft's block, its title still drawn above), not over the whole tab body; the
/// editor's exit puts the edited file back into the draft.
#[tokio::test]
async fn ctrl_e_on_a_draft_edits_over_its_claimed_rect() {
    let dir = TempDir::new().expect("a temp dir");
    let cmd = script(
        &dir,
        &format!("printf '{READY}\\n'; IFS= read -r line; printf '%s\\n' \"$line\" >> \"$1\""),
    );
    let mut harness = open().await;
    select(&mut harness, "implement");
    harness.key("e");
    for key in ["D", "R", "A", "F", "T", "space"] {
        harness.key(key);
    }
    harness.key("ctrl-e");
    let (tx, mut rx) = mpsc::channel(QUEUE);
    let file = open_editor(&mut harness, cmd, &tx);
    assert!(
        std::fs::read_to_string(&file)
            .expect("the temp file")
            .starts_with("DRAFT You are running"),
        "the editor gets the draft"
    );

    pump(
        &mut harness,
        &mut rx,
        "ready editor with the keys",
        |harness| {
            let frame = harness.render();
            frame.contains(READY) && frame.contains("Ctrl+4 to htui")
        },
    )
    .await;
    let frame = harness.render();
    let lines: Vec<&str> = frame.lines().collect();
    let title = lines
        .iter()
        .position(|line| line.contains("implement \u{b7} editing from v"))
        .unwrap_or_else(|| panic!("the draft's title stays drawn beside the pane:\n{frame}"));
    let rule = lines
        .iter()
        .position(|line| line.contains("Ctrl+4 to htui"))
        .expect("the pane's rule");
    assert_eq!(
        rule,
        title + 1,
        "the pane starts on the draft's first text row:\n{frame}"
    );
    assert!(
        lines[rule].starts_with('\u{2502}') && lines[rule + 1].starts_with('\u{2502}'),
        "the pane is inside the draft's block, not over the tab body:\n{frame}"
    );
    assert!(
        lines[rule + 1].contains(READY),
        "the editor's first row is under the rule:\n{frame}"
    );

    for key in ["x", "y", "z", "enter"] {
        harness.key(key);
    }
    pump(&mut harness, &mut rx, "editor exit", |harness| {
        !harness.app().editor_open()
    })
    .await;
    harness.settle().await;
    assert_eq!(harness.app().status, None, "the exit set no status");

    let frame = harness.render();
    assert!(
        hint(&frame).contains("Ctrl+S save"),
        "the draft is back in the view's editor: {frame}"
    );
    assert!(
        frame.contains("DRAFT You are running"),
        "the draft's own text came back: {frame}"
    );
    harness.key("ctrl-end");
    let frame = harness.render();
    assert!(
        frame.contains("xyz"),
        "the typed line is in the draft: {frame}"
    );
    assert!(!frame.contains(READY), "the pane is gone: {frame}");
    assert!(!file.exists(), "the temp file is removed");
}

/// P5: a focused editor gets `ctrl-c` (the line discipline turns it into its own SIGINT, htui
/// does not quit); `editor.focus` then `editor.abort` end it, the view hears `EDITOR_ABORTED`,
/// and the child is gone.
#[tokio::test]
async fn ctrl_c_reaches_the_editor_and_abort_ends_it() {
    let dir = TempDir::new().expect("a temp dir");
    let int = dir.path().join("int");
    let pidfile = dir.path().join("pid");
    let cmd = script(
        &dir,
        &format!(
            "trap 'printf int > \"{int}\"' INT; printf '%s' $$ > \"{pid}\"; printf {READY}; \
             while :; do sleep 0.1; done",
            int = int.display(),
            pid = pidfile.display(),
        ),
    );
    let mut harness = open().await;
    select(&mut harness, "implement");
    harness.key("E");
    let (tx, mut rx) = mpsc::channel(QUEUE);
    let file = open_editor(&mut harness, cmd, &tx);
    pump(&mut harness, &mut rx, "ready editor", |harness| {
        harness.render().contains(READY)
    })
    .await;
    let pid = read_pid(&pidfile).expect("the script wrote its pid before `ready`");

    harness.key("ctrl-c");
    assert!(!harness.app().should_quit, "ctrl-c is the editor's");
    pump(&mut harness, &mut rx, "SIGINT in the editor", |_| {
        int.exists()
    })
    .await;
    assert!(
        harness.app().editor_open(),
        "the editor survived its ctrl-c"
    );
    assert!(alive(&pid), "the editor survived its ctrl-c");

    harness.key("ctrl-4");
    harness.key("ctrl-x");
    assert!(
        !harness.app().editor_open(),
        "abort closes the editor at once"
    );
    assert!(!harness.app().should_quit);
    let frame = harness.render();
    assert!(
        notice(&frame).contains(EDITOR_ABORTED),
        "the view heard the abort: {frame}"
    );
    assert!(!file.exists(), "the temp file is removed");
    gone(&pid).await;

    // B4: the aborted editor's late events (its `Exited` at least) are inert. They are held back
    // until a second editor is open, so a late `Exited` that were not filtered by id would end
    // that one (with nothing open, `on_pane_event` has nothing to end either way).
    let second_dir = TempDir::new().expect("a temp dir");
    let second = script(&second_dir, "printf second-ready; exec sleep 30");
    harness.key("E");
    let (second_tx, mut second_rx) = mpsc::channel(QUEUE);
    open_editor(&mut harness, second, &second_tx);
    pump(
        &mut harness,
        &mut second_rx,
        "the second editor ready",
        |harness| harness.render().contains("second-ready"),
    )
    .await;
    let second_id = harness.app().editor_id();
    let mut exited = false;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
        exited |= matches!(event, PaneEvent::Exited { .. });
        assert_ne!(Some(event.id()), second_id, "the first editor's event");
        harness.app().on_pane_event(event);
    }
    assert!(exited, "the wait thread still reported the first exit");
    assert_eq!(
        harness.app().editor_id(),
        second_id,
        "a late `Exited` ends nothing"
    );
    let frame = harness.render();
    assert!(
        frame.contains("second-ready") && frame.contains("Ctrl+4 to htui"),
        "the second editor still has its pane and the keys: {frame}"
    );
}

/// PRD metric: quitting htui with a live editor leaves no child behind.
#[tokio::test]
async fn quitting_with_a_live_editor_leaves_no_child() {
    let dir = TempDir::new().expect("a temp dir");
    let pidfile = dir.path().join("pid");
    let cmd = script(
        &dir,
        &format!(
            "printf '%s' $$ > \"{pid}\"; exec sleep 30",
            pid = pidfile.display()
        ),
    );
    let mut harness = open().await;
    select(&mut harness, "implement");
    harness.key("E");
    let (tx, mut rx) = mpsc::channel(QUEUE);
    open_editor(&mut harness, cmd, &tx);
    pump(&mut harness, &mut rx, "editor pid", |_| {
        read_pid(&pidfile).is_some()
    })
    .await;
    let pid = read_pid(&pidfile).expect("the pid");
    assert!(alive(&pid), "the editor runs");

    harness.key("ctrl-4");
    harness.key("q");
    assert!(
        harness.app().should_quit,
        "`q` quits at once while htui has the keys"
    );
    drop(harness);
    gone(&pid).await;
}

//! Guards the *order*, not the behaviour: `terminal.rs` must never hand the terminal to a ratatui
//! init again, and must never stack a panic hook of its own on top of the one `init` installed.
//!
//! The behaviour half of this is already tested in `panic_hook.rs` — that binary drives the real
//! chain and proves a contained panic does not reach the restore. It cannot see `init` at all, and
//! does not have to: `init` is the only thing in the workspace that decides who installs a hook,
//! so pinning `init` pins the order. Revert `init` to `ratatui::init()` and every test in that
//! binary still passes, because the chain it exercises is the right one and nothing checks that
//! `init` is the thing that built it.
//!
//! A comment is not enough to close that, which is why the comment in `terminal.rs` is not the
//! guard: it says why a ratatui init must not come back, and a future reader who is in a hurry
//! deletes it, or writes the call anyway because the comment is three lines up. A test fails
//! instead of being argued with.
//!
//! Its own binary, and it is allowed to be its own binary, because it touches no panic hook at
//! all — the process-state rule that pins `panic_hook.rs` to exactly one `#[test]` does not apply
//! here.
//!
//! What it does not catch, stated so nobody has to find out: a call to one of these from a
//! *different* file — a new terminal setup in `lib.rs` would pass this — a name written with odd
//! spacing (`ratatui :: init`), or a hook written out by hand instead of named, since someone
//! rebuilding `set_panic_hook`'s body inline would pass this too. It pins the four names in the
//! one file that had the defect, which is the shape the defect actually took; the other three are
//! gaps to know about before trusting it, not things it tries to close.
//!
//! Modelled on `prompt_settings.rs::no_key_name_is_spelled_in_the_section`, which is the same move
//! on a section that reads its rows from a registry: the shape is what is being enforced, so the
//! test reads the shape.

/// `terminal.rs` with its comments removed.
///
/// The comments are dropped because the fix is *about* naming the functions it forbids — `init`'s
/// and `enter`'s doc comments have to be able to say `ratatui::init()` out loud, and a guard that
/// forbade the words would push the next author into writing a vaguer comment instead. So the
/// guard reads the code, which is the only half where a call can live. `terminal.rs` holds no
/// string literal containing `//`, so a first-match cut per line is the whole of it.
fn code() -> String {
    include_str!("../src/terminal.rs")
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Acceptance line 1: `init` builds the terminal itself, so htui's hook is the outermost one for
/// the whole process lifetime.
#[test]
fn terminal_rs_never_delegates_the_terminal_to_a_ratatui_init() {
    let code = code();

    assert!(
        !code.contains("ratatui::init"),
        "a `ratatui::init` call puts an unconditional `restore` in front of htui's predicate, so a \
         panic `run_providers` contains (H-20) still takes the terminal down under a running event \
         loop — the predicate is right and the terminal is gone anyway, which is exactly the \
         defect MOD-56 was opened for"
    );
    assert!(
        !code.contains("ratatui::try_init"),
        "`ratatui::try_init` installs the same hook as `ratatui::init` before it can fail, so an \
         `expect` on its result puts an unconditional restore back in front of htui's predicate — \
         the same MOD-56, reached by a different name"
    );
    assert!(
        !code.contains("init_with_options"),
        "`ratatui::init_with_options` is `ratatui::init` with a viewport argument; it installs the \
         same hook, so routing around the first two names does not route around MOD-56"
    );
    assert!(
        !code.contains("set_panic_hook"),
        "a `set_panic_hook` call stacks a hook in front of the one `init` installed, which stops \
         being the outermost link in the chain — and a stacked hook restores unconditionally, so a \
         contained panic tears the terminal down again (MOD-56)"
    );
}

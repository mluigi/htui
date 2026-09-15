//! The panic hook's one decision: whether the panic reaching it is one that ends the process.
//!
//! Its own test binary, and one `#[test]` in it, because [`std::panic::set_hook`] is **process**
//! state: a second case running in parallel in this binary would see this one's hook, and this one
//! would see its panics.
//!
//! Review finding M1. `catch_unwind` does not stop the panic hook, so a provider panic that
//! `htui_agent::excerpt::run_providers` catches, drops and records (hazard H-20) still ran
//! `ratatui::restore()` on the way through — leaving the TUI process with no alternate screen and
//! no raw mode while the event loop carried on drawing into it. The wedged-UI shape, from a defect
//! the assembler had already decided to survive.

use std::sync::{Arc, Mutex};

use htui_agent::excerpt::{PROVIDER_THREAD_PREFIX, run_providers};
use htui_core::prompt::excerpt::{
    BuiltinRanker, ExcerptCandidate, ExcerptCaps, ExcerptProvider, ExcerptRequest,
    OwnedExcerptRequest, ProviderError,
};

/// A provider whose whole behaviour is the defect H-20 exists for.
#[derive(Debug)]
struct Boom;

impl ExcerptProvider for Boom {
    fn name(&self) -> &str {
        "boom"
    }

    fn version(&self) -> &str {
        "0.1"
    }

    fn propose(&self, _req: &ExcerptRequest<'_>) -> Result<Vec<ExcerptCandidate>, ProviderError> {
        panic!("a provider that panics must not take the terminal with it");
    }
}

/// A request with no root, so nothing here touches a filesystem: the providers panic before they
/// would look.
fn request() -> OwnedExcerptRequest {
    OwnedExcerptRequest {
        item_key: "htui:MOD-2".to_owned(),
        item_body: String::new(),
        phase: "implement".to_owned(),
        document_bodies: Vec::new(),
        touched_prefixes: Vec::new(),
        changed_paths: Vec::new(),
        roots: Vec::new(),
        budget_tokens: 100_000,
        caps: ExcerptCaps {
            max_files: 12,
            file_line_cap: 400,
            head_lines: 200,
            max_file_bytes: 524_288,
        },
        scan_cap: 20_000,
        deadline: core::time::Duration::from_secs(30),
    }
}

#[test]
fn a_contained_provider_panic_leaves_the_terminal_alone() {
    // What the hook decided, per panic, in arrival order: the thread it ran on and whether it
    // would have given the terminal back. Recorded from inside a hook rather than asserted against
    // the real terminal, because "raw mode is still on" is not observable from a test process
    // whose stdout is a pipe — and because the decision *is* the fix.
    let decisions: Arc<Mutex<Vec<(String, bool)>>> = Arc::default();
    let recorder = Arc::clone(&decisions);
    // Replaces the default rather than chaining it: the three panics below are all expected, and
    // three unwind backtraces on stderr would read as a failure.
    std::panic::set_hook(Box::new(move |_info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("<unnamed>").to_owned();
        recorder
            .lock()
            .expect("the recorder is not poisoned: nothing panics while holding it")
            .push((name, htui::terminal::restores_the_terminal()));
    }));

    let owned = request();

    // 1. The spawned case: `boom` is not `providers[0]`, so it gets a thread of its own.
    let spawned: Vec<Arc<dyn ExcerptProvider>> = vec![Arc::new(BuiltinRanker), Arc::new(Boom)];
    let (_, set) = run_providers(&spawned, &owned.as_request());
    assert_eq!(
        set,
        vec!["builtin@1".to_owned(), "boom@0.1:panic".to_owned()],
        "the panic is still dropped and recorded (H-20)"
    );

    // 2. The inline case: `providers[0]` runs on *this* thread since finding M2, so a thread-name
    //    test alone would miss it. Same containment, same answer.
    let inline: Vec<Arc<dyn ExcerptProvider>> = vec![Arc::new(Boom)];
    let (_, set) = run_providers(&inline, &owned.as_request());
    assert_eq!(set, vec!["boom@0.1:panic".to_owned()]);

    // 3. And a panic that is *not* a contained one, on the same thread as case 2 — so this also
    //    proves the containment is scoped to the call and does not leak past it.
    let _ = std::panic::catch_unwind(|| panic!("the shell itself"));

    let _ = std::panic::take_hook();
    let seen = decisions.lock().expect("the recorder is not poisoned");
    assert_eq!(seen.len(), 3, "three panics, three decisions: {seen:?}");

    let (thread, restores) = &seen[0];
    assert!(
        thread.starts_with(PROVIDER_THREAD_PREFIX),
        "a provider thread is named so a backtrace says whose defect it is, got `{thread}`"
    );
    assert!(
        !restores,
        "a caught provider panic must not tear the terminal down under a running event loop"
    );

    let (_, restores) = &seen[1];
    assert!(!restores, "the inline provider is contained too");

    let (thread, restores) = &seen[2];
    assert!(
        !thread.starts_with(PROVIDER_THREAD_PREFIX),
        "the shell's own thread is not a provider thread, got `{thread}`"
    );
    assert!(
        restores,
        "a panic the process does not survive still gives the terminal back — that is the whole \
         reason the hook exists"
    );
}

//! A directory picker a section owns as a mode (MOD-49 P2): not a registered overlay, because an
//! overlay factory takes no start path and has no way to hand a chosen path back.
//!
//! It holds no store handle and no channel: listings arrive through [`PathPicker::on_reply`], and
//! requests leave as [`PickerOutcome::Request`] for the owning section to send (`R-NF-3`).
//! Navigation is lexical and never canonicalises, so nothing on screen names a link's target (P5,
//! `R-BOX-4`): the one canonicalisation is the write the section sends on [`PickerOutcome::Chosen`].

use crossterm::event::{KeyCode, KeyEvent};
use htui_core::root_path::DirListing;
use ratatui::Frame;
use ratatui::layout::Rect;

use crate::store_worker::{StoreReply, StoreRequest};
use crate::ui::{TextField, Theme};

/// What one key did (MOD-49 P8).
#[derive(Debug, Clone)]
pub enum PickerOutcome {
    /// Handled here (moved, typed, swallowed): nothing for the section to do.
    None,
    /// A listing to send through `ctx.request` — a read, never marked busy (P11).
    Request(StoreRequest),
    /// The directory the user chose, as navigated (never canonicalised; the write does that).
    Chosen(String),
    /// `Esc` outside go-to: close the picker, write nothing.
    Cancelled,
}

/// A directory picker over this box's filesystem (MOD-49).
#[derive(Debug)]
pub struct PathPicker {
    /// The popup's title, e.g. "Root of `graphics` on this box" (blueprint D22).
    title: String,
    /// The directory the header shows and `S` chooses: the last path listed or refused (D10).
    path: String,
    /// The path the newest `ListDir` named; a listing of any other path is ignored (P9).
    asked: String,
    /// The last listing of `path`, or `None` while the first one is read and after a refusal.
    listing: Option<DirListing>,
    /// The refusal of `asked`, shown in the popup (P7).
    error: Option<String>,
    /// Index into `listing.entries`.
    cursor: usize,
    /// Whether the newest request asked for `.`-entries (`.` toggles it, blueprint D1).
    show_hidden: bool,
    /// The one-line go-to field, while open (P6); its `Debug` never prints the text.
    goto: Option<TextField>,
    /// The child to highlight when the next listing lands — set by going up (D13).
    reselect: Option<String>,
}

/// The directory a picker opens on (MOD-49 P7, blueprint D9): `stored` when set, else `fallback`
/// (a repo's workspace root on this box), else `home`, else `/`. Empty strings count as unset.
#[must_use]
pub fn start_dir(stored: Option<&str>, fallback: Option<&str>, home: Option<&str>) -> String {
    let _ = (stored, fallback, home);
    todo!("MOD-49 T3")
}

impl PathPicker {
    /// A picker on `start`, and the request that lists it: `ListDir { path: start, show_hidden:
    /// false }`. `path` is `start` from the first frame, so a refused start still has a header and
    /// `h` still goes up (P7).
    #[must_use]
    pub fn open(title: impl Into<String>, start: String) -> (Self, StoreRequest) {
        let _ = (title.into(), start);
        todo!("MOD-49 T3")
    }

    /// The directory on screen.
    #[must_use]
    pub fn path(&self) -> &str {
        todo!("MOD-49 T3")
    }

    /// One key (P8, blueprint D11, D12, D15). The caller passes `CONTROL` chords on before
    /// calling this; every key not bound here is swallowed (`PickerOutcome::None`).
    pub fn on_key(&mut self, key: KeyEvent) -> PickerOutcome {
        let _ = key;
        todo!("MOD-49 T3")
    }

    /// A bracketed paste (P6, blueprint D12): into go-to when it is open, opening it otherwise.
    pub fn on_paste(&mut self, text: &str) {
        let _ = text;
        todo!("MOD-49 T3")
    }

    /// A reply addressed to the section; `true` when it was this picker's (blueprint D10).
    pub fn on_reply(&mut self, reply: &StoreReply) -> bool {
        let _ = reply;
        todo!("MOD-49 T3")
    }

    /// The popup, centred over `area` (blueprint D14).
    pub fn render(&self, frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
        let _ = (frame, area, theme);
        todo!("MOD-49 T3")
    }
}

#[cfg(test)]
mod tests {
    use super::{PathPicker, PickerOutcome, start_dir};
    use crate::store_worker::{LIST_DIR, StoreReply, StoreRequest};
    use crate::ui::Theme;
    use crate::ui::cells::cell_width;
    use crossterm::event::{KeyCode, KeyEvent};
    use htui_core::root_path::{DirEntry, DirListing};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// A key with no modifiers.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// A plain character key.
    fn ch(c: char) -> KeyEvent {
        key(KeyCode::Char(c))
    }

    /// A listing of `path` by hand: `(name, is_link)` per entry.
    fn dirs(path: &str, names: &[(&str, bool)], more: usize) -> StoreReply {
        StoreReply::DirListing(DirListing {
            path: path.to_owned(),
            entries: names
                .iter()
                .map(|(name, is_link)| DirEntry {
                    name: (*name).to_owned(),
                    is_link: *is_link,
                })
                .collect(),
            more,
        })
    }

    /// The `ListDir` an outcome carries, as `(path, show_hidden)`, or a panic naming the outcome.
    #[track_caller]
    fn asked(outcome: PickerOutcome) -> (String, bool) {
        match outcome {
            PickerOutcome::Request(StoreRequest::ListDir { path, show_hidden }) => {
                (path, show_hidden)
            }
            other => panic!("expected a listing request: {other:?}"),
        }
    }

    /// The path an outcome chose, or a panic naming the outcome.
    #[track_caller]
    fn chosen(outcome: PickerOutcome) -> String {
        match outcome {
            PickerOutcome::Chosen(path) => path,
            other => panic!("expected a choice: {other:?}"),
        }
    }

    /// Whether an outcome is `None`.
    fn none(outcome: &PickerOutcome) -> bool {
        matches!(outcome, PickerOutcome::None)
    }

    /// A picker opened on `/srv` with `htui/`, `notes/` and `shared@` listed.
    fn at_srv() -> PathPicker {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(picker.on_reply(&dirs(
            "/srv",
            &[("htui", false), ("notes", false), ("shared", true)],
            0
        )));
        picker
    }

    /// The popup over a blank 100×30 frame, one trimmed line per row.
    fn render(picker: &PathPicker) -> String {
        let mut term = Terminal::new(TestBackend::new(100, 30)).expect("a test backend");
        term.draw(|frame| picker.render(frame, frame.area(), &Theme::default()))
            .expect("the picker draws");
        let buffer = term.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            let mut line = String::new();
            let mut x = 0;
            while x < buffer.area.width {
                let symbol = buffer[(x, y)].symbol();
                line.push_str(symbol);
                x += u16::try_from(cell_width(symbol).max(1)).unwrap_or(1);
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }

    #[test]
    fn start_dir_prefers_stored_then_fallback_then_home_then_root() {
        let (s, f, h) = (Some("/stored"), Some("/root"), Some("/home/u"));
        assert_eq!(start_dir(s, f, h), "/stored");
        assert_eq!(start_dir(None, f, h), "/root");
        assert_eq!(start_dir(Some(""), f, h), "/root");
        assert_eq!(start_dir(None, None, h), "/home/u");
        assert_eq!(start_dir(None, Some(""), h), "/home/u");
        assert_eq!(start_dir(None, None, None), "/");
        assert_eq!(start_dir(None, None, Some("")), "/");
    }

    #[test]
    fn open_asks_for_the_start() {
        let (picker, request) = PathPicker::open("Root", "/srv".to_owned());
        assert_eq!(
            asked(PickerOutcome::Request(request)),
            ("/srv".to_owned(), false)
        );
        assert_eq!(picker.path(), "/srv");
    }

    /// P9: a listing of a path the picker no longer wants changes nothing.
    #[test]
    fn a_listing_for_another_path_is_ignored() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(!picker.on_reply(&dirs("/elsewhere", &[("stray", false)], 0)));
        assert_eq!(picker.path(), "/srv");
        assert!(!render(&picker).contains("stray"));
        assert!(
            none(&picker.on_key(ch('s'))),
            "nothing landed to choose from"
        );
    }

    #[test]
    fn j_and_k_clamp_at_both_ends() {
        let mut picker = at_srv();
        assert!(none(&picker.on_key(ch('k'))));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/htui");
        for _ in 0..5 {
            picker.on_key(ch('j'));
        }
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/shared");
        picker.on_key(key(KeyCode::Up));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/notes");
        picker.on_key(key(KeyCode::Down));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/shared");
    }

    #[test]
    fn enter_and_l_open_the_highlighted_directory() {
        let mut picker = at_srv();
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
        let mut picker = at_srv();
        assert_eq!(
            asked(picker.on_key(ch('l'))),
            ("/srv/htui".to_owned(), false)
        );
    }

    /// P5: up from a link is the link's lexical parent, never its target's.
    #[test]
    fn h_and_backspace_go_to_the_lexical_parent() {
        for up in [ch('h'), key(KeyCode::Backspace)] {
            let mut picker = at_srv();
            picker.on_key(ch('j'));
            picker.on_key(ch('j'));
            let (shared, _) = asked(picker.on_key(ch('l')));
            assert_eq!(shared, "/srv/shared");
            assert!(picker.on_reply(&dirs("/srv/shared", &[("inside", false)], 0)));
            assert_eq!(picker.path(), "/srv/shared");
            assert_eq!(asked(picker.on_key(up)), ("/srv".to_owned(), false));
        }
    }

    #[test]
    fn h_at_the_root_stays_at_the_root() {
        let (mut picker, _) = PathPicker::open("Root", "/".to_owned());
        assert!(picker.on_reply(&dirs("/", &[("srv", false)], 0)));
        assert!(none(&picker.on_key(ch('h'))));
        assert_eq!(picker.path(), "/");
    }

    /// Blueprint D13: back and forth keeps its place.
    #[test]
    fn going_up_highlights_the_directory_you_came_from() {
        let (mut picker, _) = PathPicker::open("Root", "/srv/notes".to_owned());
        assert!(picker.on_reply(&dirs("/srv/notes", &[], 0)));
        assert_eq!(asked(picker.on_key(ch('h'))), ("/srv".to_owned(), false));
        assert!(picker.on_reply(&dirs(
            "/srv",
            &[("htui", false), ("notes", false), ("shared", true)],
            0
        )));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/notes");
    }

    #[test]
    fn s_chooses_the_highlighted_directory() {
        let mut picker = at_srv();
        picker.on_key(ch('j'));
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/notes");
    }

    #[test]
    fn capital_s_chooses_the_listed_directory() {
        let mut picker = at_srv();
        picker.on_key(ch('j'));
        assert_eq!(chosen(picker.on_key(ch('S'))), "/srv");
    }

    /// Blueprint D11: nothing is chosen from a listing that hasn't landed or was refused.
    #[test]
    fn s_and_capital_s_need_a_listing() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(none(&picker.on_key(ch('s'))));
        assert!(none(&picker.on_key(ch('S'))));
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: "`/srv` does not exist on this box".to_owned(),
        }));
        assert!(none(&picker.on_key(ch('s'))));
        assert!(none(&picker.on_key(ch('S'))));

        // An empty listing has a directory to choose and no entry to open.
        let (mut picker, _) = PathPicker::open("Root", "/empty".to_owned());
        assert!(picker.on_reply(&dirs("/empty", &[], 0)));
        assert!(none(&picker.on_key(ch('s'))));
        assert!(none(&picker.on_key(ch('l'))));
        assert_eq!(chosen(picker.on_key(ch('S'))), "/empty");
    }

    /// P6, blueprint D12: go-to opens holding `/`, so the typed rest is absolute.
    #[test]
    fn slash_opens_go_to_and_enter_lists_what_was_typed() {
        let mut picker = at_srv();
        assert!(none(&picker.on_key(ch('/'))));
        assert!(render(&picker).contains("go to: /"));
        for c in "srv/htui".chars() {
            // Bound letters type into go-to instead of acting (`h`, `s`, `j` among them).
            assert!(none(&picker.on_key(ch(c))));
        }
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
        assert!(!render(&picker).contains("go to:"), "a submit closes go-to");
    }

    #[test]
    fn a_paste_with_go_to_closed_opens_it() {
        let mut picker = at_srv();
        picker.on_paste("/srv/htui\n");
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/srv/htui".to_owned(), false)
        );
    }

    #[test]
    fn a_paste_into_a_lone_slash_replaces_it() {
        let mut picker = at_srv();
        picker.on_key(ch('/'));
        picker.on_paste("/opt/data");
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/opt/data".to_owned(), false)
        );

        // Anywhere else the paste is inserted at the cursor.
        let mut picker = at_srv();
        picker.on_key(ch('/'));
        picker.on_key(ch('o'));
        picker.on_paste("pt");
        assert_eq!(
            asked(picker.on_key(key(KeyCode::Enter))),
            ("/opt".to_owned(), false)
        );
    }

    #[test]
    fn esc_in_go_to_closes_only_go_to() {
        let mut picker = at_srv();
        picker.on_key(ch('/'));
        assert!(none(&picker.on_key(key(KeyCode::Esc))));
        assert!(!render(&picker).contains("go to:"));
        assert!(matches!(
            picker.on_key(key(KeyCode::Esc)),
            PickerOutcome::Cancelled
        ));
    }

    /// An empty go-to submit closes go-to and asks for nothing.
    #[test]
    fn an_empty_go_to_asks_for_nothing() {
        let mut picker = at_srv();
        picker.on_key(ch('/'));
        picker.on_key(key(KeyCode::Backspace));
        assert!(none(&picker.on_key(key(KeyCode::Enter))));
        assert!(!render(&picker).contains("go to:"));
    }

    /// Blueprint D1: `.` re-asks the worker, which filters before the cap.
    #[test]
    fn dot_relists_with_hidden_shown() {
        let mut picker = at_srv();
        assert_eq!(asked(picker.on_key(ch('.'))), ("/srv".to_owned(), true));
        assert!(picker.on_reply(&dirs("/srv", &[(".git", false), ("htui", false)], 0)));
        assert_eq!(
            asked(picker.on_key(ch('l'))),
            ("/srv/.git".to_owned(), true)
        );
        assert_eq!(asked(picker.on_key(ch('.'))), ("/srv".to_owned(), false));
    }

    #[test]
    fn esc_cancels() {
        let mut picker = at_srv();
        assert!(matches!(
            picker.on_key(key(KeyCode::Esc)),
            PickerOutcome::Cancelled
        ));
    }

    /// P7: a refused listing shows its sentence in the popup, and `h` still climbs out.
    #[test]
    fn a_refused_listing_shows_its_message_and_h_still_works() {
        let (mut picker, _) = PathPicker::open("Root", "/srv/gone".to_owned());
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: "constraint violated: `/srv/gone` does not exist on this box".to_owned(),
        }));
        assert!(render(&picker).contains("`/srv/gone` does not exist on this box"));
        assert_eq!(picker.path(), "/srv/gone");
        assert_eq!(asked(picker.on_key(ch('h'))), ("/srv".to_owned(), false));
    }

    /// A refusal of another request isn't the picker's.
    #[test]
    fn another_requests_refusal_is_not_the_pickers() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        assert!(!picker.on_reply(&StoreReply::Failed {
            request: "set_repo_path",
            message: "nope".to_owned(),
        }));
        assert!(!render(&picker).contains("nope"));
    }

    /// Blueprint D15 (H-4): `q` can't quit and `?` can't open help from inside the popup.
    #[test]
    fn unbound_keys_are_swallowed() {
        let mut picker = at_srv();
        for c in ['q', '?', '1'] {
            assert!(none(&picker.on_key(ch(c))));
        }
        assert_eq!(chosen(picker.on_key(ch('s'))), "/srv/htui");
    }

    /// Blueprint D14: `name/` for a directory, `name@` for a link, `+N more`, the hint.
    #[test]
    fn the_popup_marks_links_and_counts_the_rest() {
        let (mut picker, _) = PathPicker::open("Root of `graphics` on this box", "/srv".to_owned());
        assert!(picker.on_reply(&dirs(
            "/srv",
            &[("htui", false), ("notes", false), ("shared", true)],
            3
        )));
        let frame = render(&picker);
        assert!(frame.contains("Root of `graphics` on this box"), "{frame}");
        assert!(frame.contains("/srv"), "{frame}");
        assert!(frame.contains("> htui/"), "{frame}");
        assert!(frame.contains("  notes/"), "{frame}");
        assert!(frame.contains("  shared@"), "{frame}");
        assert!(frame.contains("+3 more"), "{frame}");
        assert!(frame.contains("j/k move"), "{frame}");
        assert!(frame.contains("Esc cancel"), "{frame}");
    }

    /// The cursor stays on screen past the window, which scrolls without state.
    #[test]
    fn a_long_listing_scrolls_to_the_cursor() {
        let names: Vec<String> = (0..60).map(|n| format!("d{n:02}")).collect();
        let entries: Vec<(&str, bool)> = names.iter().map(|n| (n.as_str(), false)).collect();
        let (mut picker, _) = PathPicker::open("Root", "/many".to_owned());
        assert!(picker.on_reply(&dirs("/many", &entries, 0)));
        for _ in 0..50 {
            picker.on_key(ch('j'));
        }
        let frame = render(&picker);
        assert!(frame.contains("> d50/"), "{frame}");
        assert!(!frame.contains("d00/"), "{frame}");
    }

    /// The workspace switcher's rule: "still reading" and "nothing there" are different screens,
    /// and a refusal is a third.
    #[test]
    fn reading_and_empty_and_refused_are_three_texts() {
        let (mut picker, _) = PathPicker::open("Root", "/srv".to_owned());
        let reading = render(&picker);
        assert!(reading.contains("reading"), "{reading}");

        assert!(picker.on_reply(&dirs("/srv", &[], 0)));
        let empty = render(&picker);
        assert!(empty.contains("no directories here"), "{empty}");
        assert!(!empty.contains("reading"), "{empty}");

        picker.on_key(ch('l'));
        let (_, _) = asked(picker.on_key(ch('h')));
        assert!(picker.on_reply(&StoreReply::Failed {
            request: LIST_DIR,
            message: "`/` cannot be read on this box".to_owned(),
        }));
        let refused = render(&picker);
        assert!(refused.contains("cannot be read on this box"), "{refused}");
        assert!(!refused.contains("no directories here"), "{refused}");
    }
}

//! "Ask" selection probe (spec 2 §13.2, §13.5): select text in the app to check within 5 s; prints what CUE reads of it —
//! the kind, the length, the rectangles, read-only — and, when the field cannot say, what the Ctrl+C fallback copies (its
//! length) and whether the copy left the Win+V history (decision 27). The text itself is never printed.
//! Run: cargo run -p cue-platform-win --example selection_probe
use std::time::Duration;

use cue_platform_win::clipboard::{copy_selection, forget_in_history, WinClipboard, WinKeys};
use cue_platform_win::focus::{FocusWorker, SelectionRead};
use cue_platform_win::selection::{bounds, needs_copy_fallback};

fn main() {
    println!("Select some text in the app to check; reading in 5 s…");
    std::thread::sleep(Duration::from_secs(5));
    let worker = FocusWorker::spawn();
    let info = worker.query_with_selection(500, 8_000, Duration::from_secs(2));
    println!("app: {} · editable: {:?}", info.app, info.editable);
    println!("selection: {:?}", info.selection);
    if let SelectionRead::Text { rects, .. } = &info.selection {
        println!("lines: {rects:?}");
        println!("first line and box: {:?}", bounds(rects));
    }
    // The probe is not run in a terminal app: the terminal rule is the caller's.
    if needs_copy_fallback(&info.selection, info.editable, false) {
        let mut clip = WinClipboard::new().expect("clipboard");
        let copied = copy_selection(&mut clip, &mut WinKeys);
        println!("Ctrl+C fallback: {:?} characters", copied.as_ref().map(|t| t.chars().count()));
        if let Some(handle) = copied.and_then(forget_in_history) {
            println!("Win+V history: {:?}", handle.join());
        }
    }
}

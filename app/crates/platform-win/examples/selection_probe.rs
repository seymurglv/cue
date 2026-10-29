//! "Ask" selection probe (spec 2 §13.2, §13.5): select text in the app to check within 5 s; prints what CUE reads of it —
//! the kind, the length, the rectangles, read-only — and, when the field cannot say, what the Ctrl+C fallback copies (its
//! length), whether the copy left the Win+V history (decision 27), and whether a late copy was caught. The text itself is
//! never printed.
//! Run: cargo run -p cue-platform-win --example selection_probe
use std::time::Duration;

use cue_platform_win::clipboard::{
    cloud_clipboard_uploads, copy_selection, forget_in_history, spawn_late_guard, WinClipboard, WinKeys,
};
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
    // The probe is not run in a terminal app: the terminal rule is the caller's (and the copy's own check). With Windows
    // uploading copies to the cloud clipboard there is no copy, as in the app.
    let cloud = cloud_clipboard_uploads();
    println!("copies uploaded to the cloud clipboard: {cloud}");
    if needs_copy_fallback(&info.selection, info.editable, false, cloud)
        && let Some(target) = info.window
    {
        let mut clip = WinClipboard::new().expect("clipboard");
        let mut late_guard = None;
        let copy = copy_selection(&mut clip, &mut WinKeys, target, |late| late_guard = spawn_late_guard(late));
        println!(
            "Ctrl+C fallback: {:?} characters, restore failed: {}",
            copy.text.as_ref().map(|t| t.chars().count()),
            copy.restore_failed
        );
        if let Some(handle) = copy.text.and_then(|text| forget_in_history(text, copy.since)) {
            println!("Win+V history: {:?}", handle.join());
        }
        if let Some(handle) = late_guard {
            println!("late copy (watched for 2 s): {:?}", handle.join());
        }
    }
}

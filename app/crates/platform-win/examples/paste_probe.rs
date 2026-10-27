//! After 3 s pastes a line at the caret. Run: cargo run -p cue-platform-win --example paste_probe
use std::cell::Cell;

use cue_platform_win::clipboard::{paste_text, WinClipboard, WinKeys};

fn main() {
    println!("Put the caret in Notepad — pasting in 3 s.");
    std::thread::sleep(std::time::Duration::from_secs(3));
    let mut clip = match WinClipboard::new() {
        Ok(clip) => clip,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let pasted = Cell::new(false);
    let text = "CUE paste check — русский, English, 25 %.\nSecond line (line break LF → CRLF).";
    let result = paste_text(&mut clip, &mut WinKeys, text, || pasted.set(true));
    println!("{result:?}, on_pasted: {}", pasted.get());
}

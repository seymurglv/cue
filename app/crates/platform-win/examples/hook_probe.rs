//! Probe 2: prints chord events. Run: cargo run -p cue-platform-win --example hook_probe
use std::sync::atomic::Ordering;

fn main() {
    let (tx, rx) = std::sync::mpsc::channel();
    let _thread = match cue_platform_win::hook::spawn(tx) {
        Ok(thread) => thread,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    cue_platform_win::hook::ESC_ACTIVE.store(true, Ordering::Relaxed);
    // As if a recording were always on: every fresh Shift press prints `ModeToggle`.
    cue_platform_win::hook::RECORDING.store(true, Ordering::Relaxed);
    println!("Hook installed.");
    println!("Press Ctrl+Win, Ctrl+Win+V, Ctrl+Win+D, Ctrl+Win+arrows, Shift, Esc. To exit, close the terminal window.");
    for ev in rx {
        println!("{ev:?}");
    }
}

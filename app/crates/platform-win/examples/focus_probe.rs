//! Every 2 s prints where the text would go. Run: cargo run -p cue-platform-win --example focus_probe [-- N]
//! N — how many queries to make (without it — until closed). The text before the caret is not printed — only its length.
//! At the end of the line — how long the request took.
use std::time::{Duration, Instant};

fn main() {
    let rounds = match std::env::args().nth(1).map(|a| a.parse::<u64>()) {
        None => u64::MAX,
        Some(Ok(n)) => n,
        Some(Err(_)) => {
            eprintln!("the number of queries must be an integer: focus_probe [N]");
            std::process::exit(2);
        }
    };
    let w = cue_platform_win::focus::FocusWorker::spawn();
    for _ in 0..rounds {
        std::thread::sleep(Duration::from_secs(2));
        let t = Instant::now();
        let i = w.query(500, Duration::from_millis(150));
        let ms = t.elapsed().as_millis();
        let before = match &i.before_cursor {
            Some(s) => format!("{} chars", s.chars().count()),
            None => "none".into(),
        };
        println!("{} | {:?} | password: {} | before caret: {before} | {ms} ms", i.app, i.editable, i.is_password);
    }
}

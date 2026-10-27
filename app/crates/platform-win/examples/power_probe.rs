//! Prints resume, suspend, lock and unlock. Run:
//! cargo run -p cue-platform-win --example power_probe [-- N]
//! N — how many seconds to listen (without it — until closed). Check: Win+L → sign in again, sleep → wake up.
//! At the start of the line — seconds since the probe started by the Windows clock (sleep time counts too).
//! The module's warnings (window not created, lock not tracked) are printed here as well.
use std::time::{Duration, SystemTime};

fn main() {
    tracing_subscriber::fmt().init();
    let secs = match std::env::args().nth(1).map(|a| a.parse::<u64>()) {
        None => None,
        Some(Ok(n)) => Some(n),
        Some(Err(_)) => {
            eprintln!("the number of seconds must be an integer: power_probe [N]");
            std::process::exit(2);
        }
    };
    println!("Listening: Win+L, unlock, sleep and wake… To exit, close the terminal window.");
    let start = SystemTime::now();
    cue_platform_win::power::spawn(move |event| {
        let at = SystemTime::now().duration_since(start).unwrap_or_default().as_secs();
        println!("{at:>6} s | {event:?}");
    });
    match secs {
        Some(n) => std::thread::sleep(Duration::from_secs(n)),
        None => loop {
            std::thread::park();
        },
    }
}

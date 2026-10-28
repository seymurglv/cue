//! Shows what the Drop's z-order guard sees. Run while CUE is running:
//! cargo run -p cue-platform-win --example zorder_probe [-- N]
//! Lists the visible windows above the Drop window (class name and process id, never titles): always on top or not,
//! cloaked, overlapping the Drop. Then the guard's verdict: "covered" means the Drop would be put back on top.
//! N — repeat every 0.5 s for N seconds, printing only changes of the verdict.
use std::time::{Duration, Instant};

use cue_platform_win::dropwin::{self, Above, Seen};
use cue_platform_win::placement::PxRect;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, GW_HWNDPREV, GWL_EXSTYLE, GetClassNameW, GetWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
    IsWindowVisible, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::w;

fn main() {
    let secs = match std::env::args().nth(1).map(|a| a.parse::<u64>()) {
        None => None,
        Some(Ok(n)) => Some(n),
        Some(Err(_)) => {
            eprintln!("the number of seconds must be an integer: zorder_probe [N]");
            std::process::exit(2);
        }
    };
    let Some(drop) = find_drop() else {
        eprintln!("the Drop window was not found: is CUE running?");
        std::process::exit(1);
    };
    let Seen::Shown { rect, .. } = dropwin::observe(drop) else {
        eprintln!("the Drop window is hidden");
        std::process::exit(1);
    };
    println!("Drop window {:?}: {rect:?}", drop.0);
    let above = dropwin::windows_above(drop);
    for (w, hwnd) in above.iter().zip(handles_above(drop)) {
        let overlap = if w.rect.intersects(&rect) { "overlaps" } else { "" };
        let kind = if w.topmost { "on top" } else { "ordinary" };
        let cloaked = if w.cloaked { "cloaked" } else { "" };
        println!("  {:<40} pid {:>6}  {kind:<8} {cloaked:<7} {overlap}", class_of(hwnd), pid_of(hwnd));
    }
    let verdict = |r: PxRect, a: &[Above]| if dropwin::covered_by_ordinary_window(r, a) { "covered" } else { "fine" };
    println!("{} windows above; verdict: {}", above.len(), verdict(rect, &above));
    let Some(n) = secs else { return };
    let start = Instant::now();
    let mut last = verdict(rect, &above);
    while start.elapsed() < Duration::from_secs(n) {
        std::thread::sleep(Duration::from_millis(500));
        let Seen::Shown { rect, .. } = dropwin::observe(drop) else { continue };
        let now = verdict(rect, &dropwin::windows_above(drop));
        if now != last {
            println!("{:>6.1} s | {now}", start.elapsed().as_secs_f64());
            last = now;
        }
    }
}

/// A top-level window titled "CUE" with the Drop's style (tool window, never activated).
fn find_drop() -> Option<HWND> {
    let mut after = None;
    loop {
        // SAFETY: a search through the top-level windows by title; no window is changed.
        let hwnd = unsafe { FindWindowExW(None, after, None, w!("CUE")) }.ok()?;
        let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 && ex & WS_EX_NOACTIVATE.0 != 0 {
            return Some(hwnd);
        }
        after = Some(hwnd);
    }
}

/// The same walk as `dropwin::windows_above`, for the handles (visible windows only, in the same order).
fn handles_above(drop: HWND) -> Vec<HWND> {
    let mut out = Vec::new();
    let mut cur = drop;
    // SAFETY: GetWindow and IsWindowVisible only read the window list.
    while let Ok(h) = unsafe { GetWindow(cur, GW_HWNDPREV) } {
        if h.0.is_null() {
            break;
        }
        if unsafe { IsWindowVisible(h) }.as_bool() {
            out.push(h);
        }
        cur = h;
    }
    out
}

fn class_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: the buffer is ours and its length is passed in.
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0;
    // SAFETY: reads the owner process id into our variable.
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

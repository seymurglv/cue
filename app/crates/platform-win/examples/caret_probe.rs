//! Join space: what CUE sees before the caret and whether it puts a space before the paste.
//! Run: cargo run -p cue-platform-win --example caret_probe [-- --every S] [-- --field NAME]
//!
//! Without `--every` reads lines from standard input: each line is a case label, end of input — exit.
//! With `--every S` queries the focused field every S seconds until closed (the label is the query number).
//! One line per query: label, app, how many characters CUE sees before the caret, the last (up to 3) of
//! them and the join decision; then the element type, `FrameworkId` and "caret at line start" according to UI Automation.
//! The text itself is not printed: a letter or digit is `w`, anything else is a code (`U+000A`).
//!
//! `--field NAME` — for a test setup: query only when the focused field has this accessible name (for example,
//! the test page's `aria-label`; checked before and after the query), and then also the line breaks in the whole field —
//! their positions and codes. Another field in focus or the name cannot be read — the query is skipped; UI Automation
//! for the name check did not start — the probe exits with an error at once (a foreign field is never queried on any failure).
use std::io::BufRead;
use std::time::{Duration, Instant};

use cue_core::refine::join_for_insert;
use cue_platform_win::focus::FocusWorker;

/// As when CUE pastes (`INSERT_CONTEXT_CHARS`, `FOCUS_TIMEOUT` in `runtime.rs`).
const INSERT_CONTEXT_CHARS: usize = 16;
const FOCUS_TIMEOUT: Duration = Duration::from_millis(150);

/// Characters without the text: a letter or digit is `w`, anything else is a code.
fn codes(s: &str, last: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let tail = &chars[chars.len().saturating_sub(last)..];
    let shown: Vec<String> =
        tail.iter().map(|c| if c.is_alphanumeric() { "w".into() } else { format!("U+{:04X}", *c as u32) }).collect();
    format!("[{}]", shown.join(" "))
}

struct Args {
    every: Option<Duration>,
    field: Option<String>,
}

fn args() -> Args {
    let usage = || -> ! {
        eprintln!("caret_probe [--every SECONDS] [--field NAME]");
        std::process::exit(2);
    };
    let mut a = Args { every: None, field: None };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        match (k.as_str(), it.next()) {
            ("--every", Some(v)) => a.every = Some(Duration::from_secs(v.parse().unwrap_or_else(|_| usage()))),
            ("--field", Some(v)) => a.field = Some(v),
            _ => usage(),
        }
    }
    a
}

fn main() {
    let a = args();
    let w = FocusWorker::spawn();
    #[cfg(windows)]
    let raw = raw::Raw::new();
    // `--field` guards against reading a foreign field: if the field name cannot be checked, nothing is queried at all.
    #[cfg(windows)]
    let can_check = raw.is_some();
    #[cfg(not(windows))]
    let can_check = false;
    if a.field.is_some() && !can_check {
        eprintln!("--field: UI Automation for the field check did not start — the probe does not query");
        std::process::exit(1);
    }
    let labels: Box<dyn Iterator<Item = String>> = match a.every {
        Some(every) => Box::new((1..).map(move |i| {
            std::thread::sleep(every);
            i.to_string()
        })),
        None => Box::new(std::io::stdin().lock().lines().map_while(Result::ok)),
    };
    for label in labels {
        #[cfg(windows)]
        let ours = |raw: &raw::Raw| a.field.as_deref().is_none_or(|f| raw.focused_name().as_deref() == Some(f));
        #[cfg(windows)]
        if raw.as_ref().is_some_and(|r| !ours(r)) {
            println!("{label} | the focus is not on the probe's field — not queried");
            continue;
        }
        let t = Instant::now();
        let info = w.query(INSERT_CONTEXT_CHARS, FOCUS_TIMEOUT);
        let ms = t.elapsed().as_millis();
        let (seen, n) = match &info.before_cursor {
            Some(s) => (codes(s, 3), s.chars().count()),
            None => ("none".into(), 0),
        };
        let space = join_for_insert(info.before_cursor.as_deref(), "Probe").starts_with(' ');
        let space = if space { "SPACE" } else { "no space" };
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut out = format!(
            "{label} | {} | {:?} | before caret {n} chars, end {seen} | {space} | {ms} ms",
            info.app, info.editable
        );
        #[cfg(windows)]
        if let Some(raw) = &raw {
            let facts = raw.facts(a.field.is_some());
            if !ours(raw) {
                println!("{label} | the focus moved during the query — the query does not count");
                continue;
            }
            out.push_str(" || ");
            out.push_str(&facts);
        }
        println!("{out}");
    }
}

#[cfg(windows)]
mod raw {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomationTextPattern, IUIAutomationTextRange, TextPatternRangeEndpoint_End,
        TextPatternRangeEndpoint_Start, TextUnit, TextUnit_Character, TextUnit_Line, TextUnit_Paragraph,
        UIA_TextPatternId,
    };
    use windows::core::Result;

    use super::codes;

    /// UI Automation of the probe's main thread (`FocusWorker` has its own, on its thread).
    pub struct Raw {
        uia: IUIAutomation,
    }

    impl Raw {
        pub fn new() -> Option<Self> {
            // SAFETY: COM of this (main) thread; the UI Automation object is created and used on it.
            unsafe {
                CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
                let uia = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
                Some(Self { uia })
            }
        }

        /// Accessible name of the focused element.
        pub fn focused_name(&self) -> Option<String> {
            // SAFETY: as in `read`.
            unsafe { self.uia.GetFocusedElement().and_then(|el| el.CurrentName()) }.ok().map(|n| n.to_string())
        }

        /// Facts about the caret without the text; `whole` — also the line break positions in the whole field (only for the probe's field).
        pub fn facts(&self, whole: bool) -> String {
            self.read(whole).unwrap_or_else(|e| format!("UIA error {}", e.code()))
        }

        fn read(&self, whole: bool) -> Result<String> {
            // SAFETY: calls on our own UI Automation object and live elements on the thread where it was created.
            unsafe {
                let el = self.uia.GetFocusedElement()?;
                let ct = el.CurrentControlType()?.0;
                let fw = el.CurrentFrameworkId()?.to_string();
                let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) else {
                    return Ok(format!("type {ct} {fw:?}: no text pattern"));
                };
                let sel = tp.GetSelection()?;
                if sel.Length()? < 1 {
                    return Ok(format!("type {ct} {fw:?}: no selection"));
                }
                let caret = sel.GetElement(0)?;
                let at = caret.Clone()?;
                at.MoveEndpointByRange(TextPatternRangeEndpoint_End, &caret, TextPatternRangeEndpoint_Start)?;
                let line = at_unit_start(&at, TextUnit_Line)?;
                let mut out = format!("type {ct} {fw:?}, line start {line}");
                if whole {
                    // One character before the caret (UIA itself, not limited to the field) and the paragraph start.
                    let prev = at.Clone()?;
                    let moved = prev.MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1)?;
                    let prev_text = prev.GetText(8)?.to_string();
                    let para = at_unit_start(&at, TextUnit_Paragraph)?;
                    let doc = tp.DocumentRange()?.GetText(4096)?.to_string();
                    let breaks: Vec<String> = doc
                        .chars()
                        .enumerate()
                        .filter(|(_, c)| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}' | '\u{FFFC}'))
                        .map(|(i, c)| format!("{i}:U+{:04X}", c as u32))
                        .collect();
                    out.push_str(&format!(
                        ", paragraph start {para}, char before caret (moved {moved}) {}, field {} chars, breaks [{}]",
                        codes(&prev_text, 3),
                        doc.chars().count(),
                        breaks.join(" "),
                    ));
                }
                Ok(out)
            }
        }
    }

    /// Whether the start of the `unit` enclosing the empty range `at` coincides with it.
    unsafe fn at_unit_start(at: &IUIAutomationTextRange, unit: TextUnit) -> Result<bool> {
        // SAFETY: as in `read`.
        unsafe {
            let r = at.Clone()?;
            r.ExpandToEnclosingUnit(unit)?;
            Ok(r.CompareEndpoints(TextPatternRangeEndpoint_Start, at, TextPatternRangeEndpoint_Start)? == 0)
        }
    }
}

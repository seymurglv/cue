### Task 6: the field watcher

**Files:**
- Create: `app/crates/platform-win/src/fieldwatch.rs`
- Modify: `app/crates/platform-win/src/lib.rs` (`pub mod fieldwatch;` in order, not behind `cfg(windows)`)

**Interfaces:**
- Consumes: `cue_core::corrections::{Step, Watch, READ_EVERY_MS}`.
- Produces: `pub trait FieldReader { fn read(&mut self) -> Option<String>; }`,
  `pub trait Pace { fn now_ms(&mut self) -> u64; fn sleep(&mut self, d: Duration); }`,
  `pub fn run(reader: &mut impl FieldReader, pasted: &str, stop: &AtomicBool, pace: &mut impl Pace) -> Option<String>`;
  on Windows `pub fn spawn(pasted: String, stop: Arc<AtomicBool>, done: impl FnOnce(Option<String>) + Send + 'static) -> bool`
  (false: the thread did not start).

- [ ] **Step 1: write the failing tests** (pure, run on any system):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct Script(Vec<Option<&'static str>>);
    impl FieldReader for Script {
        fn read(&mut self) -> Option<String> {
            if self.0.is_empty() { None } else { self.0.remove(0).map(str::to_owned) }
        }
    }
    #[derive(Default)]
    struct Clock { now: u64, slept: u32 }
    impl Pace for Clock {
        fn now_ms(&mut self) -> u64 { self.now }
        fn sleep(&mut self, d: Duration) { self.now += d.as_millis() as u64; self.slept += 1; }
    }

    #[test]
    fn reads_until_the_correction_settles() {
        let mut field = Script(
            [vec![Some("позвони марине"), Some("позвони Марго")], vec![Some("позвони Марго"); 10]].concat(),
        );
        let (stop, mut clock) = (AtomicBool::new(false), Clock::default());
        assert_eq!(run(&mut field, "позвони марине", &stop, &mut clock), Some("позвони Марго".into()));
        assert!(clock.now >= 400 + cue_core::corrections::SETTLE_MS, "{}", clock.now);
    }

    #[test]
    fn a_stop_ends_with_the_span_so_far() {
        struct Stopper<'a>(&'a AtomicBool, u32);
        impl FieldReader for Stopper<'_> {
            fn read(&mut self) -> Option<String> {
                self.1 += 1;
                if self.1 == 2 { self.0.store(true, Ordering::Relaxed); }
                Some(if self.1 == 1 { "позвони марине" } else { "позвони Марго" }.into())
            }
        }
        let stop = AtomicBool::new(false);
        let mut field = Stopper(&stop, 0);
        assert_eq!(run(&mut field, "позвони марине", &stop, &mut Clock::default()), Some("позвони Марго".into()));
    }

    #[test]
    fn an_unreadable_field_ends_at_once() {
        let mut clock = Clock::default();
        assert_eq!(run(&mut Script(vec![None]), "текст", &AtomicBool::new(false), &mut clock), None);
        assert_eq!(clock.slept, 0);
    }
}
```

- [ ] **Step 2: run and see them fail** — `cargo test -p cue-platform-win fieldwatch` (check the package name in
  `app/crates/platform-win/Cargo.toml`).

- [ ] **Step 3: implement the pure part**

```rust
//! The field CUE pasted into, read again after the paste to notice the user's correction
//! (docs/specs/2026-11-02-corrections.md). The loop is plain code over a reader; on Windows the reader is the
//! UI Automation element focused at the paste, kept and read on a thread of its own. Its text stays in memory.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cue_core::corrections::{Step, Watch, READ_EVERY_MS};

/// The field kept at the paste. `None`: it can no longer be read.
pub trait FieldReader {
    fn read(&mut self) -> Option<String>;
}

/// The watch's clock: milliseconds since it started, and the wait between readings.
pub trait Pace {
    fn now_ms(&mut self) -> u64;
    fn sleep(&mut self, d: Duration);
}

/// Reads the field until the watch is over or `stop` is set (a new dictation), and gives the corrected span.
pub fn run(reader: &mut impl FieldReader, pasted: &str, stop: &AtomicBool, pace: &mut impl Pace) -> Option<String> {
    let mut watch = Watch::new(pasted);
    loop {
        if stop.load(Ordering::Relaxed) {
            return watch.finish();
        }
        let text = reader.read();
        match watch.read(text.as_deref(), pace.now_ms()) {
            Step::Done(span) => return span,
            Step::Go => pace.sleep(Duration::from_millis(READ_EVERY_MS)),
        }
    }
}
```

- [ ] **Step 4: run the tests and see them pass.**

- [ ] **Step 5: implement the Windows reader and `spawn`** (same file):

```rust
#[cfg(windows)]
mod win {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationTextPattern, IUIAutomationValuePattern, UIA_TextPatternId,
        UIA_ValuePatternId,
    };
    use windows::core::Interface;

    use super::{FieldReader, Pace, run};

    /// The longest field text read; a longer field is cut, its end no longer matches and nothing is offered.
    const MAX_FIELD_CHARS: i32 = 20_000;
    const UIA_TIMEOUT_MS: u32 = 1_000;

    struct Com;
    impl Drop for Com {
        fn drop(&mut self) {
            // SAFETY: pairs the CoInitializeEx of this thread in `open`.
            unsafe { CoUninitialize() };
        }
    }

    /// The element focused right after the paste, by its text pattern or else its value pattern. Created and used on
    /// the watch's own thread only.
    struct UiaField {
        text: Option<IUIAutomationTextPattern>,
        value: Option<IUIAutomationValuePattern>,
        _uia: IUIAutomation,
        _com: Com,
    }

    impl FieldReader for UiaField {
        fn read(&mut self) -> Option<String> {
            // SAFETY: calls on our own UI Automation objects on the thread where they were created.
            unsafe {
                if let Some(tp) = &self.text {
                    return tp.DocumentRange().ok()?.GetText(MAX_FIELD_CHARS).ok().map(|b| b.to_string());
                }
                self.value.as_ref()?.CurrentValue().ok().map(|b| b.to_string())
            }
        }
    }

    /// The focused field, when it can be read: not a password, not CUE's own window, with a text or value pattern.
    fn open() -> Option<UiaField> {
        // SAFETY: a COM apartment for this thread only; `Com` ends it with the thread.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().ok()?;
        let com = Com;
        // SAFETY: plain COM calls on this thread's apartment.
        unsafe {
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
            if let Ok(uia2) = uia.cast::<IUIAutomation2>() {
                let _ = uia2.SetConnectionTimeout(UIA_TIMEOUT_MS);
                let _ = uia2.SetTransactionTimeout(UIA_TIMEOUT_MS);
            }
            let el = uia.GetFocusedElement().ok()?;
            if !matches!(el.CurrentIsPassword().map(|b| b.as_bool()), Ok(false)) {
                return None;
            }
            if el.CurrentProcessId().ok().and_then(|p| u32::try_from(p).ok()) == Some(std::process::id()) {
                return None;
            }
            let text = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).ok();
            let value = if text.is_none() { el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok() } else { None };
            (text.is_some() || value.is_some()).then_some(UiaField { text, value, _uia: uia, _com: com })
        }
    }

    struct Clock(Instant);
    impl Pace for Clock {
        fn now_ms(&mut self) -> u64 {
            self.0.elapsed().as_millis() as u64
        }
        fn sleep(&mut self, d: Duration) {
            std::thread::sleep(d);
        }
    }

    /// Watches the focused field on the thread `cue-field`; `done` gets the corrected span (or `None`) once it is over.
    pub fn spawn(pasted: String, stop: Arc<AtomicBool>, done: impl FnOnce(Option<String>) + Send + 'static) -> bool {
        std::thread::Builder::new()
            .name("cue-field".into())
            .spawn(move || {
                let span = open().and_then(|mut field| run(&mut field, &pasted, &stop, &mut Clock(Instant::now())));
                done(span);
            })
            .inspect_err(|e| tracing::warn!("correction watch not started: {e}"))
            .is_ok()
    }
}

#[cfg(windows)]
pub use win::spawn;
```

The method names follow `focus.rs` (`GetFocusedElement`, `CurrentIsPassword`, `CurrentProcessId`,
`GetCurrentPatternAs`); `IUIAutomationValuePattern::CurrentValue` is new here and returns a `BSTR`. If 0.62's
`GetCurrentPatternAs` or `CurrentProcessId` differ in shape, copy the exact call from `focus.rs`.

- [ ] **Step 6: a live check** behind `#[ignore]` in `mod win`, run by hand:

```rust
    #[cfg(test)]
    mod tests {
        /// By hand: `cargo test -p cue-platform-win fieldwatch -- --ignored --nocapture`, then put the caret in a
        /// text field within 2 s and type into it; the span printed is the field's text.
        #[test]
        #[ignore]
        fn reads_the_focused_field() {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let mut f = super::open().expect("a readable field is focused");
            println!("{:?}", super::FieldReader::read(&mut f).map(|t| t.chars().count()));
        }
    }
```

- [ ] **Step 7: run** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`: green.

---


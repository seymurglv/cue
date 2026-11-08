### Task 2: the watch (anchors, timing, the corrected span)

**Files:**
- Modify: `app/crates/core/src/corrections.rs`

**Interfaces:**
- Produces: `pub const READ_EVERY_MS: u64 = 400`, `SETTLE_MS: u64 = 2_500`, `WATCH_MS: u64 = 60_000`,
  `ANCHOR_MS: u64 = 2_000`; `pub enum Step { Go, Done(Option<String>) }` (Debug prints only `Done(chars)`);
  `pub struct Watch` with `pub fn new(pasted: &str) -> Self`, `pub fn read(&mut self, text: Option<&str>, at: u64) -> Step`,
  `pub fn finish(&self) -> Option<String>`. `at` is milliseconds since the watch started.

- [ ] **Step 1: write the failing tests** (add to `mod tests`):

```rust
    fn go(w: &mut Watch, text: &str, at: u64) {
        assert!(matches!(w.read(Some(text), at), Step::Go), "{at}");
    }
    fn done(w: &mut Watch, text: Option<&str>, at: u64) -> Option<String> {
        match w.read(text, at) {
            Step::Done(span) => span,
            Step::Go => panic!("still watching at {at}"),
        }
    }

    #[test]
    fn the_span_is_what_replaced_the_paste_between_the_anchors() {
        let mut w = Watch::new("позвонить Марисоль.");
        go(&mut w, "Привет! Мне нужно позвонить Марисоль. Пока", 0);
        go(&mut w, "Привет! Мне нужно позвонить Мари. Пока", 400);
        go(&mut w, "Привет! Мне нужно позвонить Marisol. Пока", 800);
        go(&mut w, "Привет! Мне нужно позвонить Marisol. Пока", 3_200);
        assert_eq!(done(&mut w, Some("Привет! Мне нужно позвонить Marisol. Пока"), 3_300), Some("позвонить Marisol.".into()));
    }

    #[test]
    fn nothing_changed_is_no_correction() {
        let mut w = Watch::new("готово");
        go(&mut w, "готово", 0);
        go(&mut w, "готово", 30_000);
        assert_eq!(done(&mut w, Some("готово"), WATCH_MS), None);
    }

    #[test]
    fn a_sent_field_counts_with_its_last_text() {
        let mut w = Watch::new("позвони марине");
        go(&mut w, "позвони марине", 0);
        go(&mut w, "позвони Марго", 400);
        assert_eq!(done(&mut w, Some(""), 800), Some("позвони Марго".into()));
    }

    #[test]
    fn an_unreadable_field_ends_the_watch_with_its_last_text() {
        let mut w = Watch::new("позвони марине");
        go(&mut w, "позвони марине", 0);
        go(&mut w, "позвони Марго", 400);
        assert_eq!(done(&mut w, None, 800), Some("позвони Марго".into()));
    }

    #[test]
    fn a_field_changed_around_the_paste_gives_no_span() {
        let mut w = Watch::new("позвони марине");
        go(&mut w, "Текст. позвони марине", 0);
        go(&mut w, "Другой. позвони Марго", 400);
        assert_eq!(done(&mut w, Some("Другой. позвони Марго"), 3_000), None);
    }

    #[test]
    fn a_paste_never_seen_in_the_field_ends_the_watch() {
        let mut w = Watch::new("позвони марине");
        go(&mut w, "", 0);
        go(&mut w, "что-то своё", 1_600);
        assert_eq!(done(&mut w, Some("что-то своё"), ANCHOR_MS), None);
    }

    #[test]
    fn line_breaks_are_compared_as_one_kind() {
        let mut w = Watch::new("раз\nдва");
        go(&mut w, "раз\r\nдва", 0);
        go(&mut w, "раз\r\nDva2", 400);
        assert_eq!(done(&mut w, Some("раз\r\nDva2"), 2_900), Some("раз\nDva2".into()));
    }

    #[test]
    fn finish_gives_the_span_so_far() {
        let mut w = Watch::new("позвони марине");
        go(&mut w, "позвони марине", 0);
        assert_eq!(w.finish(), None);
        go(&mut w, "позвони Марго", 400);
        assert_eq!(w.finish(), Some("позвони Марго".into()));
    }

    #[test]
    fn an_empty_paste_is_not_watched() {
        assert_eq!(done(&mut Watch::new("  "), Some("что угодно"), 0), None);
    }
```

- [ ] **Step 2: run them and see them fail**

Run: `cargo test -p cue-core corrections`. Expected: compile errors, `Watch` and `Step` are not defined.

- [ ] **Step 3: implement**

```rust
/// How often the field is read while watching.
pub const READ_EVERY_MS: u64 = 400;
/// The correction is over this long after the field's last change.
pub const SETTLE_MS: u64 = 2_500;
/// The watch never lasts longer than this.
pub const WATCH_MS: u64 = 60_000;
/// The pasted text must show in the field by then, or there is nothing to watch.
pub const ANCHOR_MS: u64 = 2_000;

/// What a watch does after a reading: read again, or stop with the corrected span (`None`: no correction).
pub enum Step {
    Go,
    Done(Option<String>),
}

impl std::fmt::Debug for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Go => f.write_str("Go"),
            Self::Done(span) => f.debug_tuple("Done").field(&span.as_ref().map(|s| s.chars().count())).finish(),
        }
    }
}

/// The text of one field after a paste: where the paste sits (the text before and after it are the anchors), the last
/// text read and when it last changed. It lives in memory only.
pub struct Watch {
    pasted: String,
    anchors: Option<(String, String)>,
    last: String,
    changed_at: Option<u64>,
}

fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

impl Watch {
    pub fn new(pasted: &str) -> Self {
        Self { pasted: normalize(pasted).trim().to_owned(), anchors: None, last: String::new(), changed_at: None }
    }

    /// One reading of the field (`None`: it can no longer be read), `at` ms after the watch started.
    pub fn read(&mut self, text: Option<&str>, at: u64) -> Step {
        if self.pasted.is_empty() {
            return Step::Done(None);
        }
        let Some(text) = text.map(normalize) else { return Step::Done(self.finish()) };
        if self.anchors.is_none() {
            if let Some(i) = text.rfind(&self.pasted) {
                self.anchors = Some((text[..i].to_owned(), text[i + self.pasted.len()..].to_owned()));
                self.last = text;
                return Step::Go;
            }
            return if at >= ANCHOR_MS { Step::Done(None) } else { Step::Go };
        }
        if text.is_empty() {
            return Step::Done(self.finish());
        }
        if text != self.last {
            self.last = text;
            self.changed_at = Some(at);
        }
        let settled = self.changed_at.is_some_and(|c| at.saturating_sub(c) >= SETTLE_MS);
        if settled || at >= WATCH_MS { Step::Done(self.finish()) } else { Step::Go }
    }

    /// The corrected span so far: the last text between the anchors, when it changed and the anchors still hold.
    pub fn finish(&self) -> Option<String> {
        let (before, after) = self.anchors.as_ref()?;
        self.changed_at?;
        let t = &self.last;
        let fits = t.len() >= before.len() + after.len() && t.starts_with(before.as_str()) && t.ends_with(after.as_str());
        let span = fits.then(|| t[before.len()..t.len() - after.len()].trim().to_owned())?;
        (!span.is_empty() && span != self.pasted).then_some(span)
    }
}
```

- [ ] **Step 4: run the tests and see them pass**

Run: `cargo test -p cue-core corrections`. Expected: all pass. Byte slicing is safe: `starts_with`/`ends_with` and
`rfind` give char boundaries.

- [ ] **Step 5: clippy** — `cargo clippy -p cue-core --all-targets -- -D warnings`, clean.

---


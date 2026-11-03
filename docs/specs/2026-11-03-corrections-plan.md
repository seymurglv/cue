# Corrections and dictionary suggestions: implementation plan

**Goal:** CUE notices a word the user corrects right after a paste and offers it for the dictionary from the Drop
(capsule book · word · +, a missed one waits as a green dot).

**Architecture:** Everything that can be plain code over strings lives in `cue-core::corrections`: the word diff and
the candidate rules, the watch (anchors, timing, the corrected span) and the offer's state. `cue-platform-win::fieldwatch`
runs the watch on a thread of its own against a `FieldReader`, with UI Automation as the Windows reader. The runtime
starts the watch after a dictation's paste, turns its span into an offer, shows it when the Drop is at rest and applies
the page's answers as setting changes. The Drop page grows the capsule from the Drop's own parts (drop.js), like the
approved mockup (design stage 3, variants B + C).

**Tech stack:** Rust 2024 (workspace `app/`), `windows` 0.62 (UI Automation), Tauri 2, TypeScript + vitest (jsdom).

**Spec:** `docs/specs/2026-11-02-corrections.md`. Read it first; this plan argues from it.

## Global constraints

- Files are in English and say nothing about the tools the code was written with.
- No commits unless the owner asks; each task ends with the checks green, not with a commit.
- The field's text never reaches disk or the log: the log gets "correction found / none" and numbers only. Every new
  `Debug` that could hold a word prints its length (see `SettingChange`'s hand-written `Debug`).
- `learn.enabled` off: CUE never reads a field after a paste.
- Texts in English and Russian; the Drop's capsule has no text besides the word, its buttons have accessible names
  "Add to dictionary" / «Добавить в словарь» and "Undo" / «Отменить».
- Motion follows the family: parts open with `max-width .5s var(--e-spring)`, rings deplete linearly, reduced motion
  is respected by the existing drop.js rules.
- Checks, from `C:\Users\seymurglv\Desktop\cue-dev`: `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`; from `app`: `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`.

## File map

| File | Change |
|---|---|
| `app/crates/core/src/corrections.rs` | new: diff, candidates, `Watch`, `Offers`, `SuggestView` |
| `app/crates/core/src/lib.rs` | `pub mod corrections;` |
| `app/crates/core/src/ui.rs` | four `UiAction`s |
| `app/crates/core/src/settings.rs` | `LearnedWord`, `dictionary.learned`, `LearnSettings`, `DictLearn`, `Learn` |
| `app/crates/core/src/refine.rs` | `terms_prompt` takes the learned words |
| `app/crates/engines/src/{chat,claude,openrouter}.rs` | `terms: &[String]` → `dict: &DictionarySettings` on the refine path |
| `app/crates/platform-win/src/fieldwatch.rs` | new: `FieldReader`, `Pace`, `run`, Windows reader and `spawn` |
| `app/crates/platform-win/src/lib.rs` | `pub mod fieldwatch;` |
| `app/src-tauri/src/runtime.rs` | watch after a paste, `CoreEvent::Corrected`, offers, actions |
| `app/src-tauri/src/ipc.rs` | `Ui::suggest`, `Ui::settings`, `SettingsView.learned` / `.learn` |
| `app/ui/src/design/drop.js`, `drop.css` | capsule parts, states `sugg` / `added`, the dot |
| `app/ui/src/controller.ts`, `strings.ts`, `main.ts` | `suggest()`, its actions, `cue://suggest` |
| `app/ui/src/window/{api,page,dictionary,settings,text}.ts`, `design/window.css` | green book, hint, switch, `cue://settings` |
| `README.md`, `docs/ARCHITECTURE.md` | the feature and its privacy line |

---

### Task 1: the word diff and the candidate rules

**Files:**
- Create: `app/crates/core/src/corrections.rs`
- Modify: `app/crates/core/src/lib.rs` (add `pub mod corrections;` in alphabetical order)

**Interfaces:**
- Produces: `pub struct Replacement { pub heard: String, pub word: String }` (Clone, PartialEq, Eq; hand-written
  `Debug` with lengths only), `pub fn candidates(pasted: &str, corrected: &str, dictionary: &[String]) -> Vec<Replacement>`,
  `pub const MAX_TOKENS: usize = 400`.

- [ ] **Step 1: write the failing tests** at the end of `corrections.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn found(pasted: &str, corrected: &str) -> Vec<(String, String)> {
        candidates(pasted, corrected, &[]).into_iter().map(|r| (r.heard, r.word)).collect()
    }
    fn pair(heard: &str, word: &str) -> (String, String) {
        (heard.into(), word.into())
    }

    #[test]
    fn a_name_written_in_latin_is_a_candidate() {
        assert_eq!(found("Мне нужно позвонить Марисоль.", "Мне нужно позвонить Marisol."), [pair("Марисоль", "Marisol")]);
    }

    #[test]
    fn several_heard_words_can_become_one() {
        assert_eq!(found("открой кубер нетис сейчас", "открой Kubernetes сейчас"), [pair("кубер нетис", "Kubernetes")]);
    }

    #[test]
    fn case_and_punctuation_alone_are_no_change() {
        assert!(found("позвони github", "позвони GitHub").is_empty());
        assert!(found("привет мир", "привет, мир!").is_empty());
    }

    #[test]
    fn deleted_and_inserted_words_are_not_candidates() {
        assert!(found("это очень важно", "это важно").is_empty());
        assert!(found("это важно", "это очень важно").is_empty());
    }

    #[test]
    fn a_run_longer_than_three_words_is_not_a_candidate() {
        assert!(found("мы пойдём туда завтра утром рано", "мы Go To The Park рано").is_empty());
    }

    #[test]
    fn ordinary_lowercase_words_are_not_candidates() {
        assert!(found("позвони маме", "позвони папе").is_empty());
    }

    #[test]
    fn a_capital_counts_inside_a_sentence_but_not_at_its_start() {
        assert_eq!(found("позвони марине", "позвони Марго"), [pair("марине", "Марго")]);
        assert!(found("Марина придёт.", "Марго придёт.").is_empty(), "start of the text");
        assert!(found("Тест готов. Марина придёт.", "Тест готов. Марго придёт.").is_empty(), "after a full stop");
        assert_eq!(found("Готово. гитхаб открыт.", "Готово. GitHub открыт."), [pair("гитхаб", "GitHub")], "another sign");
    }

    #[test]
    fn a_capital_inside_or_a_digit_makes_a_term() {
        assert_eq!(found("open the git hub repo", "open the GitHub repo"), [pair("git hub", "GitHub")]);
        assert_eq!(found("ask gpt four", "ask GPT-4"), [pair("gpt four", "GPT-4")]);
        assert!(found("version three", "version 3").is_empty(), "no letter: not a term");
    }

    #[test]
    fn cyrillic_in_latin_text_is_a_candidate_too() {
        assert_eq!(found("call the shop pyaterochka now", "call the shop Пятёрочка now"), [pair("pyaterochka", "Пятёрочка")]);
    }

    #[test]
    fn a_word_already_in_the_dictionary_is_not_offered() {
        let dict = ["marisol".to_string()];
        assert!(candidates("позвонить Марисоль", "позвонить Marisol", &dict).is_empty());
    }

    #[test]
    fn outer_punctuation_stays_out_of_the_word() {
        assert_eq!(found("звонила «Марисоль», да", "звонила «Marisol», да"), [pair("Марисоль", "Marisol")]);
    }

    #[test]
    fn very_long_texts_are_not_diffed() {
        let long = "слово ".repeat(MAX_TOKENS + 1);
        assert!(found(&long, &format!("{long} Marisol")).is_empty());
    }

    #[test]
    fn debug_shows_no_words() {
        let r = Replacement { heard: "Марисоль".into(), word: "Marisol".into() };
        let d = format!("{r:?}");
        assert!(!d.contains("Marisol") && !d.contains("Марисоль"), "{d}");
    }
}
```

- [ ] **Step 2: run them and see them fail**

Run: `cargo test -p cue-core corrections` (from `C:\Users\seymurglv\Desktop\cue-dev`). Expected: compile errors,
`candidates` and `Replacement` are not defined.

- [ ] **Step 3: write the module's top**

```rust
//! Corrections after a paste (docs/specs/2026-11-02-corrections.md): which replaced words could go into the
//! dictionary, how a watched field's text becomes the corrected span, and the one suggestion the Drop offers.
//! Plain code over strings: the platform reads the field, the runtime shows the suggestion.

use crate::refine;

/// The most words a pasted or a corrected text may have for the diff; a longer one is not looked at.
pub const MAX_TOKENS: usize = 400;
/// The longest run of words, on either side, that counts as one replacement.
const MAX_RUN: usize = 3;
/// Punctuation trimmed off a word's ends before it is compared or offered (not `+`, `#`, `-`: C++, C#, GPT-4).
const EDGE: &[char] = &['.', ',', ';', ':', '!', '?', '…', '"', '\'', '«', '»', '„', '“', '”', '(', ')', '[', ']'];
/// A word right after one of these starts a sentence.
const SENTENCE_END: &[char] = &['.', '!', '?', '…'];

/// A replacement the user made: what was heard and pasted, and the word they wrote instead.
#[derive(Clone, PartialEq, Eq)]
pub struct Replacement {
    pub heard: String,
    pub word: String,
}

impl std::fmt::Debug for Replacement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Replacement")
            .field("heard_chars", &self.heard.chars().count())
            .field("word_chars", &self.word.chars().count())
            .finish()
    }
}

/// One replaced run of words, before the candidate rules.
struct Change {
    heard: String,
    word: String,
    /// The run starts the corrected text or follows a sentence's end.
    sentence_start: bool,
}

fn clean(s: &str) -> &str {
    s.trim_matches(EDGE)
}

fn key(token: &str) -> String {
    clean(token).to_lowercase()
}

/// The replaced runs of one to three words on each side, aligned word by word (the longest common subsequence of the
/// words compared without case and outer punctuation). Deleted and inserted words are not runs.
fn changes(pasted: &str, corrected: &str) -> Vec<Change> {
    let a: Vec<&str> = pasted.split_whitespace().collect();
    let b: Vec<&str> = corrected.split_whitespace().collect();
    if a.len() > MAX_TOKENS || b.len() > MAX_TOKENS {
        return Vec::new();
    }
    let ka: Vec<String> = a.iter().map(|t| key(t)).collect();
    let kb: Vec<String> = b.iter().map(|t| key(t)).collect();
    // lcs[i][j]: how many words a[i..] and b[j..] have in common, in order.
    let mut lcs = vec![vec![0u16; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if ka[i] == kb[j] { lcs[i + 1][j + 1] + 1 } else { lcs[i + 1][j].max(lcs[i][j + 1]) };
        }
    }
    let (mut i, mut j, mut gi, mut gj) = (0, 0, 0, 0);
    let mut out = Vec::new();
    loop {
        let end = i == a.len() && j == b.len();
        if end || (i < a.len() && j < b.len() && ka[i] == kb[j]) {
            let (old, new) = (&a[gi..i], &b[gj..j]);
            if !old.is_empty() && !new.is_empty() && old.len() <= MAX_RUN && new.len() <= MAX_RUN {
                let sentence_start = gj == 0 || b[gj - 1].trim_end_matches(['"', '»', '”', ')']).ends_with(SENTENCE_END);
                out.push(Change { heard: clean(&old.join(" ")).to_owned(), word: clean(&new.join(" ")).to_owned(), sentence_start });
            }
            if end {
                break;
            }
            i += 1;
            j += 1;
            (gi, gj) = (i, j);
        } else if j == b.len() || (i < a.len() && lcs[i + 1][j] >= lcs[i][j + 1]) {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}
```

- [ ] **Step 4: write the candidate rules**

```rust
/// Whether `word` looks like a name or a term: a capital inside it (GitHub, New York's second word), a digit with a
/// letter (GPT-4), letters of the other alphabet than the text around it, or a capital first letter where no sentence
/// starts. It must hold a letter.
fn looks_like_name(word: &str, sentence_start: bool, text: &str) -> bool {
    if !word.chars().any(char::is_alphabetic) {
        return false;
    }
    let inner_capital = word.chars().skip(1).any(char::is_uppercase);
    let digit = word.chars().any(|c| c.is_ascii_digit());
    let first_capital = word.chars().next().is_some_and(char::is_uppercase) && !sentence_start;
    inner_capital || digit || first_capital || other_alphabet(word, text)
}

fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic()
}

fn is_cyrillic(c: char) -> bool {
    matches!(c, '\u{0400}'..='\u{04FF}')
}

/// The word holds Latin letters in a mostly Cyrillic text, or Cyrillic ones in a mostly Latin text (the word's own
/// letters are counted out of the text).
fn other_alphabet(word: &str, text: &str) -> bool {
    let count = |s: &str, f: fn(char) -> bool| s.chars().filter(|&c| f(c)).count();
    let (lat, cyr) = (count(text, is_latin) - count(word, is_latin), count(text, is_cyrillic) - count(word, is_cyrillic));
    (word.chars().any(is_latin) && cyr > lat) || (word.chars().any(is_cyrillic) && lat > cyr)
}

/// The replacements worth offering for the dictionary, in the text's order: a name or a term (see
/// [`looks_like_name`]) that the dictionary does not hold in any case and that it can take (`refine::is_term`).
pub fn candidates(pasted: &str, corrected: &str, dictionary: &[String]) -> Vec<Replacement> {
    let mut out: Vec<Replacement> = Vec::new();
    for c in changes(pasted, corrected) {
        let lower = c.word.to_lowercase();
        let known = dictionary.iter().any(|t| t.trim().to_lowercase() == lower);
        let repeat = out.iter().any(|r| r.word.to_lowercase() == lower);
        if known || repeat || key(&c.heard) == lower || !refine::is_term(&c.word) || !refine::is_term(&c.heard) {
            continue;
        }
        if looks_like_name(&c.word, c.sentence_start, corrected) {
            out.push(Replacement { heard: c.heard, word: c.word });
        }
    }
    out
}
```

- [ ] **Step 5: run the tests and see them pass**

Run: `cargo test -p cue-core corrections`. Expected: all pass. If `other_alphabet` underflows on a word that is not in
the text, the tests catch it: the word always comes from `corrected`, so the counts cannot go below zero; keep the
subtraction as written.

- [ ] **Step 6: clippy**

Run: `cargo clippy -p cue-core --all-targets -- -D warnings`. Expected: clean.

---

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

### Task 3: the offer's state and the Drop's actions

**Files:**
- Modify: `app/crates/core/src/corrections.rs`, `app/crates/core/src/ui.rs`

**Interfaces:**
- Produces in `corrections`: `#[derive(Clone, PartialEq, Eq, serde::Serialize)] pub struct SuggestView { pub word: String, pub missed: bool }`
  (hand-written `Debug`: `missed` and the word's length); `#[derive(Default)] pub struct Offers` with
  `offer(&mut self, r: Replacement)`, `at_rest(&mut self) -> bool` (true: the view changed, send it),
  `view(&self) -> Option<SuggestView>`, `dismissed(&mut self) -> bool`, `added(&mut self) -> Option<Replacement>`,
  `undone(&mut self) -> Option<Replacement>`, `done(&mut self) -> bool`, `dictation_started(&mut self) -> bool`.
- Produces in `ui`: `UiAction::{SuggestAdd, SuggestUndo, SuggestDismiss, SuggestDone}` (JSON `suggestAdd`,
  `suggestUndo`, `suggestDismiss`, `suggestDone`).

- [ ] **Step 1: write the failing tests** (in `corrections` tests):

```rust
    fn marisol() -> Replacement {
        Replacement { heard: "Марисоль".into(), word: "Marisol".into() }
    }
    fn shown(o: &Offers) -> Option<(String, bool)> {
        o.view().map(|v| (v.word, v.missed))
    }

    #[test]
    fn an_offer_waits_for_rest_then_shows() {
        let mut o = Offers::default();
        o.offer(marisol());
        assert_eq!(shown(&o), None, "waiting");
        assert!(o.at_rest());
        assert_eq!(shown(&o), Some(("Marisol".into(), false)));
        assert!(!o.at_rest(), "already shown");
    }

    #[test]
    fn a_dismissed_offer_is_missed_until_the_next_dictation() {
        let mut o = Offers::default();
        o.offer(marisol());
        o.at_rest();
        assert!(o.dismissed());
        assert_eq!(shown(&o), Some(("Marisol".into(), true)));
        assert!(o.dictation_started());
        assert_eq!(shown(&o), None);
        assert!(!o.at_rest(), "gone for good");
    }

    #[test]
    fn adding_then_undoing_gives_the_word_back_once() {
        let mut o = Offers::default();
        o.offer(marisol());
        o.at_rest();
        assert_eq!(o.added(), Some(marisol()));
        assert_eq!(o.added(), None, "once");
        assert_eq!(o.undone(), Some(marisol()));
        assert_eq!(shown(&o), None);
    }

    #[test]
    fn a_missed_offer_can_still_be_added_from_the_hover() {
        let mut o = Offers::default();
        o.offer(marisol());
        o.at_rest();
        o.dismissed();
        assert_eq!(o.added(), Some(marisol()));
        assert!(o.done());
        assert_eq!(shown(&o), None);
    }

    #[test]
    fn one_offer_at_a_time() {
        let mut o = Offers::default();
        o.offer(marisol());
        o.at_rest();
        o.offer(Replacement { heard: "гитхаб".into(), word: "GitHub".into() });
        assert_eq!(shown(&o), Some(("Marisol".into(), false)), "an open one stays");
        o.dismissed();
        o.offer(Replacement { heard: "гитхаб".into(), word: "GitHub".into() });
        assert_eq!(shown(&o), None, "a new one replaces a missed one and waits for rest");
        o.at_rest();
        assert_eq!(shown(&o), Some(("GitHub".into(), false)));
    }

    #[test]
    fn a_dictation_keeps_a_waiting_offer_and_drops_a_shown_one() {
        let mut o = Offers::default();
        o.offer(marisol());
        assert!(!o.dictation_started());
        o.at_rest();
        assert!(o.dictation_started());
        assert_eq!(shown(&o), None);
    }

    #[test]
    fn answers_out_of_turn_change_nothing() {
        let mut o = Offers::default();
        assert!(!o.dismissed() && !o.done());
        assert_eq!((o.added(), o.undone()), (None, None));
        o.offer(marisol());
        assert_eq!(o.added(), None, "not shown yet");
    }

    #[test]
    fn the_view_goes_to_the_page_as_word_and_missed() {
        let v = SuggestView { word: "Marisol".into(), missed: true };
        assert_eq!(serde_json::to_value(&v).unwrap(), serde_json::json!({ "word": "Marisol", "missed": true }));
        assert!(!format!("{v:?}").contains("Marisol"));
    }
```

And in `ui.rs`'s `every_variant_has_its_json_name` test add to the `actions` array:
`(UiAction::SuggestAdd, "suggestAdd"), (UiAction::SuggestUndo, "suggestUndo"), (UiAction::SuggestDismiss, "suggestDismiss"), (UiAction::SuggestDone, "suggestDone")`
(follow the array's existing shape exactly; if it lists names only, add the four names).

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core`; expected: `Offers`, `SuggestView` and the
  four variants are not defined.

- [ ] **Step 3: implement `Offers`**

```rust
/// What the Drop shows of the suggestion: the word, and whether it was missed (the dot by the idle bar).
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub struct SuggestView {
    pub word: String,
    pub missed: bool,
}

impl std::fmt::Debug for SuggestView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SuggestView").field("chars", &self.word.chars().count()).field("missed", &self.missed).finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Found while the Drop was busy: shown at the next rest.
    Waiting,
    /// The capsule with its 8 s ring.
    Open,
    /// The word went in; the capsule shows "undo" with its 6 s ring.
    Added,
    /// The ring ran out: the dot waits until the next dictation.
    Missed,
}

/// The one suggestion at a time.
#[derive(Default)]
pub struct Offers {
    cur: Option<(Replacement, Stage)>,
}

impl Offers {
    /// A new candidate: it replaces a waiting or a missed one; an open or just added one stays.
    pub fn offer(&mut self, r: Replacement) {
        if !matches!(self.cur, Some((_, Stage::Open | Stage::Added))) {
            self.cur = Some((r, Stage::Waiting));
        }
    }

    /// The Drop is at rest: a waiting suggestion opens. True when the view changed.
    pub fn at_rest(&mut self) -> bool {
        self.step(Stage::Waiting, Stage::Open)
    }

    pub fn view(&self) -> Option<SuggestView> {
        match &self.cur {
            Some((r, Stage::Open | Stage::Added)) => Some(SuggestView { word: r.word.clone(), missed: false }),
            Some((r, Stage::Missed)) => Some(SuggestView { word: r.word.clone(), missed: true }),
            _ => None,
        }
    }

    /// The 8 s ring ran out.
    pub fn dismissed(&mut self) -> bool {
        self.step(Stage::Open, Stage::Missed)
    }

    /// "+": the word to put into the dictionary, once.
    pub fn added(&mut self) -> Option<Replacement> {
        match &mut self.cur {
            Some((r, stage @ (Stage::Open | Stage::Missed))) => {
                *stage = Stage::Added;
                Some(r.clone())
            }
            _ => None,
        }
    }

    /// "Undo": the word to take out again; the suggestion is over.
    pub fn undone(&mut self) -> Option<Replacement> {
        match self.cur.take() {
            Some((r, Stage::Added)) => Some(r),
            other => {
                self.cur = other;
                None
            }
        }
    }

    /// The 6 s ring after adding ran out: the suggestion is over.
    pub fn done(&mut self) -> bool {
        let added = matches!(self.cur, Some((_, Stage::Added)));
        if added {
            self.cur = None;
        }
        added
    }

    /// The microphone opened for a dictation: a shown or missed suggestion goes (an added word stays in the
    /// dictionary); a waiting one keeps waiting. True when the view changed.
    pub fn dictation_started(&mut self) -> bool {
        let shown = matches!(self.cur, Some((_, Stage::Open | Stage::Added | Stage::Missed)));
        if shown {
            self.cur = None;
        }
        shown
    }

    fn step(&mut self, from: Stage, to: Stage) -> bool {
        match &mut self.cur {
            Some((_, stage)) if *stage == from => {
                *stage = to;
                true
            }
            _ => false,
        }
    }
}
```

In `ui.rs` add to `UiAction` after `TrayClose`, with doc comments:

```rust
    /// The suggestion's "+": the word goes into the dictionary.
    SuggestAdd,
    /// "Undo" after "+": the word comes out again.
    SuggestUndo,
    /// The suggestion's 8 s ring ran out: it waits as a dot.
    SuggestDismiss,
    /// The 6 s ring after "+" ran out.
    SuggestDone,
```

The runtime's `match` on `UiAction` has a catch-all arm today, so nothing else breaks; Task 7 adds their own arms.

- [ ] **Step 4: run** `cargo test -p cue-core` and `cargo clippy -p cue-core --all-targets -- -D warnings`: green.

---

### Task 4: settings — learned words and the switch

**Files:**
- Modify: `app/crates/core/src/settings.rs`

**Interfaces:**
- Produces: `#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)] pub struct LearnedWord { pub word: String, pub heard: String }`;
  `DictionarySettings.learned: Vec<LearnedWord>` (newest first, not written while empty);
  `pub struct LearnSettings { enabled: Option<bool> }` with `pub fn enabled(&self) -> bool` (default true);
  `Settings.learn: LearnSettings`; `SettingChange::DictLearn(LearnedWord)` (JSON `{"kind":"dictLearn","value":{"word":…,"heard":…}}`)
  and `SettingChange::Learn(bool)` (JSON `{"kind":"learn","value":false}`); `DictRemove` also removes from `learned`.

- [ ] **Step 1: write the failing tests** next to `dictionary_words_are_added_first_and_removed`:

```rust
    fn learned(word: &str, heard: &str) -> LearnedWord {
        LearnedWord { word: word.into(), heard: heard.into() }
    }

    /// A word from a suggestion goes into both lists, first; it is refused like a typed one, and when what was heard
    /// cannot go into the request; removing the word takes it out of both.
    #[test]
    fn a_learned_word_joins_the_dictionary_with_how_it_was_heard() {
        let mut s = Settings::default();
        assert!(s.apply(SettingChange::DictAdd("Soniox".into())));
        assert!(s.apply(SettingChange::DictLearn(learned(" Marisol ", " Марисоль "))));
        assert_eq!(s.dictionary.terms, ["Marisol", "Soniox"]);
        assert_eq!(s.dictionary.learned, [learned("Marisol", "Марисоль")]);
        assert!(!s.apply(SettingChange::DictLearn(learned("marisol", "марисоль"))), "already there");
        assert!(!s.apply(SettingChange::DictLearn(learned("GitHub", "a\nb"))), "heard on two lines");
        assert!(!s.apply(SettingChange::DictLearn(learned("", "пусто"))));
        assert!(s.apply(SettingChange::DictRemove("Marisol".into())));
        assert!(s.dictionary.learned.is_empty() && s.dictionary.terms == ["Soniox"]);
        let change: SettingChange =
            serde_json::from_value(serde_json::json!({ "kind": "dictLearn", "value": { "word": "GitHub", "heard": "гитхаб" } })).unwrap();
        assert_eq!(change, SettingChange::DictLearn(learned("GitHub", "гитхаб")));
    }

    #[test]
    fn learning_is_on_by_default_and_only_off_is_written() {
        let mut s = Settings::default();
        assert!(s.learn.enabled());
        assert!(s.apply(SettingChange::Learn(false)));
        assert!(!s.learn.enabled());
        assert_eq!(serde_json::to_value(&s).unwrap()["learn"], serde_json::json!({ "enabled": false }));
        assert!(!s.apply(SettingChange::Learn(false)), "no change");
        assert!(s.apply(SettingChange::Learn(true)));
        assert!(serde_json::to_value(&s).unwrap().get("learn").is_none(), "the default is not written");
        for odd in [r#"{"learn":5}"#, r#"{"learn":{"enabled":"no"}}"#, r#"{"learn":{"enabled":null}}"#] {
            let s: Settings = serde_json::from_str(odd).unwrap();
            assert!(s.learn.enabled(), "{odd}");
        }
    }

    #[test]
    fn learned_words_read_leniently_and_an_empty_list_is_not_written() {
        let s: Settings = serde_json::from_str(
            r#"{"dictionary":{"terms":["Marisol"],"learned":[{"word":"Marisol","heard":"Марисоль"},{"word":1},"x"]}}"#,
        )
        .unwrap();
        assert_eq!(s.dictionary.learned, [learned("Marisol", "Марисоль")]);
        let s: Settings = serde_json::from_str(r#"{"dictionary":{"terms":["A"],"learned":7}}"#).unwrap();
        assert!(s.dictionary.learned.is_empty());
        let mut s = Settings::default();
        s.dictionary.terms = vec!["A".into()];
        assert!(serde_json::to_value(&s).unwrap()["dictionary"].get("learned").is_none());
    }
```

Extend `debug_shows_no_device_name` (or the test that checks `DictAdd`'s `Debug`) with
`SettingChange::DictLearn(learned("Marisol", "Марисоль"))`: its `Debug` must contain neither word. Extend
`setting_change_reads_the_window_json` with the two new kinds in its existing shape.

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core settings`.

- [ ] **Step 3: implement**

Next to `DictionarySettings`:

```rust
/// A word added from a suggestion after a correction, with what recognition heard instead; refine is told both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LearnedWord {
    pub word: String,
    pub heard: String,
}

fn learned_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<LearnedWord>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else { return Ok(Vec::new()) };
    Ok(items.into_iter().filter_map(|item| serde_json::from_value(item).ok()).collect())
}
```

Add to `DictionarySettings` (after `terms`), and update its doc comment's last sentence to "…recognition does not take
them. `learned` holds the words added from a suggestion, newest first; each is in `terms` too.":

```rust
    #[serde(deserialize_with = "learned_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub learned: Vec<LearnedWord>,
```

Next to `AskSettings`:

```rust
/// "Learn from me": after a paste CUE reads the field to notice a correction (docs/specs/2026-11-02-corrections.md).
/// On by default; only `false` is written.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnSettings {
    #[serde(deserialize_with = "off_or_nothing", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl LearnSettings {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}
```

In `Settings`, after `dictionary`:

```rust
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "LearnSettings::is_default")]
    pub learn: LearnSettings,
```

and `learn: LearnSettings::default(),` in `impl Default for Settings`.

In `SettingChange`, after `DictRemove`:

```rust
    /// A word from a suggestion: first in the dictionary, with how it was heard; refused like `DictAdd`, and when
    /// `heard` could not go into the request.
    DictLearn(LearnedWord),
    /// "Learn from me" on or off.
    Learn(bool),
```

`Debug` arms:

```rust
            Self::DictLearn(l) => f
                .debug_struct("DictLearn")
                .field("chars", &l.word.chars().count())
                .field("heard_chars", &l.heard.chars().count())
                .finish(),
            Self::Learn(on) => f.debug_tuple("Learn").field(on).finish(),
```

In `apply`, move `DictAdd`'s refusal into a helper both arms use, and widen `DictRemove`:

```rust
            SettingChange::DictAdd(word) => self.dictionary.put_first(word.trim()),
            SettingChange::DictLearn(l) => {
                let (word, heard) = (l.word.trim(), l.heard.trim());
                if !refine::is_term(heard) || !self.dictionary.put_first(word) {
                    return false;
                }
                self.dictionary.learned.retain(|x| x.word.to_lowercase() != word.to_lowercase());
                self.dictionary.learned.insert(0, LearnedWord { word: word.to_owned(), heard: heard.to_owned() });
                true
            }
            SettingChange::DictRemove(word) => {
                let d = &mut self.dictionary;
                let (terms, learned) = (d.terms.len(), d.learned.len());
                d.terms.retain(|t| *t != word);
                d.learned.retain(|l| l.word != word);
                d.terms.len() != terms || d.learned.len() != learned
            }
            SettingChange::Learn(on) => set_if_changed(&mut self.learn.enabled, (!on).then_some(false)),
```

```rust
impl DictionarySettings {
    /// A word first in `terms`: refused when empty, not one line, too long, there in any case, or the list is full.
    fn put_first(&mut self, word: &str) -> bool {
        let taken = self.terms.iter().any(|t| t.trim().to_lowercase() == word.to_lowercase());
        if !refine::is_term(word) || taken || self.terms.len() >= refine::MAX_TERMS {
            return false;
        }
        self.terms.insert(0, word.to_owned());
        true
    }
}
```

(`impl DictionarySettings` already exists for `is_default`; add `put_first` there.)

- [ ] **Step 4: run** `cargo test -p cue-core` and clippy: green. `cargo test --workspace` will fail to compile in
  `app/crates/cli/src/leak.rs` or `ipc.rs` only if they build `DictionarySettings` with a struct literal; fix any such
  literal with `..Default::default()`.

- [ ] **Step 5: leak-scan sees learned words.** In `app/crates/cli/src/leak.rs` next to the loop over
  `settings.dictionary.terms` (line ~175), add the same loop over `settings.dictionary.learned` checking both `word`
  and `heard` with the same `Kind::DictionaryTerm(i)` (index continuing after the terms). Run its tests:
  `cargo test -p cue-cli` (check the crate's name in `app/crates/cli/Cargo.toml`).

---

### Task 5: refine hears how learned words sound

**Files:**
- Modify: `app/crates/core/src/refine.rs`, `app/crates/engines/src/claude.rs`, `chat.rs`, `openrouter.rs`,
  `app/src-tauri/src/runtime.rs` (lines ~1049 and ~1142), `app/crates/engines/examples/*.rs` that call these.

**Interfaces:**
- Consumes: `settings::{DictionarySettings, LearnedWord}` (Task 4).
- Produces: `pub fn terms_prompt(terms: &[String], learned: &[LearnedWord]) -> Option<String>`; on the refine path every
  `terms: &[String]` parameter becomes `dict: &DictionarySettings`.

- [ ] **Step 1: write the failing test** in refine.rs's dictionary tests:

```rust
    #[test]
    fn learned_words_say_how_they_sound() {
        let learned = [LearnedWord { word: "Marisol".into(), heard: "Марисоль".into() }];
        let block = terms_prompt(&terms(&["Soniox", "Marisol"]), &learned).unwrap();
        assert_eq!(block, format!("{TERMS_INTRO}\n\nSoniox\nMarisol — sounds like «Марисоль»"));
        let orphan = [LearnedWord { word: "Gone".into(), heard: "гон".into() }];
        assert_eq!(terms_prompt(&terms(&["Soniox"]), &orphan).unwrap(), format!("{TERMS_INTRO}\n\nSoniox"), "only words in the list");
    }
```

and change the existing calls `terms_prompt(&…)` in refine.rs and openrouter.rs tests to `terms_prompt(&…, &[])`.

- [ ] **Step 2: run and see it fail** — `cargo test -p cue-core refine`.

- [ ] **Step 3: implement in refine.rs**

```rust
/// The dictionary block: a second system block after the unchanged `SYSTEM_PROMPT`, one term per line; a word added
/// from a suggestion also says how recognition heard it. `None` when no usable term is left: the request is then
/// exactly the one without a dictionary.
pub fn terms_prompt(terms: &[String], learned: &[LearnedWord]) -> Option<String> {
    let usable = usable_terms(terms);
    let line = |t: &str| match learned.iter().find(|l| l.word.trim() == t) {
        Some(l) => format!("{t} — sounds like «{}»", l.heard.trim()),
        None => t.to_owned(),
    };
    (!usable.is_empty()).then(|| format!("{TERMS_INTRO}\n\n{}", usable.iter().map(|t| line(t)).collect::<Vec<_>>().join("\n")))
}
```

with `use crate::settings::LearnedWord;` (refine.rs and settings.rs already refer to each other; that is fine in one
crate).

- [ ] **Step 4: carry the dictionary down the refine path.** Change each parameter `terms: &[String]` in these
  functions to `dict: &DictionarySettings` (import `cue_core::settings::DictionarySettings`), and inside each body use
  `&dict.terms` where `terms` was used, except `system_blocks`, which calls `terms_prompt(&dict.terms, &dict.learned)`:
  - `engines/src/claude.rs`: `request_body` (57), `request_body_for` (64), `request_body_styled` (69),
    `system_blocks` (103), `warm_body` (136), `decide` (209), `decide_translation` (224), the functions at 254, 270,
    309, `settle` (352), `warm` (396), `warm_at` (401);
  - `engines/src/chat.rs`: 38, `refine` (112), 122, 135;
  - `engines/src/openrouter.rs`: `request_body` (188), `request_body_for` (194), 206, `refine` (279), 289, 321.

  `refine::sanity_ok` and `translate::translation_ok` keep `terms: &[String]`; their callers pass `&dict.terms`. In the
  engines' tests build the argument with `&DictionarySettings { terms: …, ..Default::default() }` (a small helper
  `fn dict(t: &[&str]) -> DictionarySettings` in each test module keeps the calls short). In `runtime.rs` replace
  `self.settings.dictionary.terms.clone()` with `self.settings.dictionary.clone()` at both places and pass `&dict` on.
  The compiler lists any call left; follow it until `cargo build --workspace --all-targets` is clean.

- [ ] **Step 5: run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`: green.
  The request with no learned words must stay byte for byte the same (the existing request-body tests prove it).

---

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

### Task 7: the runtime — watch, offer, answers, settings to the windows

**Files:**
- Modify: `app/src-tauri/src/runtime.rs`, `app/src-tauri/src/ipc.rs`

**Interfaces:**
- Consumes: Tasks 1–6.
- Produces: `CoreEvent::Corrected { id: DictationId, pasted: String, span: Option<String> }`; `Ui::suggest(&self, v: Option<&SuggestView>)`
  emitting `cue://suggest` to the Drop; `Ui::settings(&self, v: &SettingsView)` emitting `cue://settings` to every
  window; `SettingsView.learned: Vec<LearnedView>` (`{ word, heard }`, camelCase) and `SettingsView.learn: bool`.

- [ ] **Step 1: `SettingsView` gets the learned words and the switch** (ipc.rs). Write the test first: in
  `settings_view_json_names` add `"learn"` and `"learned"` to the sorted key list and `("learn", json!(true))`,
  `("learned", json!([]))` to the defaults table; run `cargo test -p cue` (the app crate's name in
  `app/src-tauri/Cargo.toml`) and see it fail. Then add:

```rust
    /// The dictionary's words added from a suggestion, with how they were heard (the green book's tooltip).
    pub learned: Vec<LearnedView>,
    /// "Learn from me".
    pub learn: bool,
```

```rust
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct LearnedView {
    pub word: String,
    pub heard: String,
}
```

In `SettingsView::of`: `learned: s.dictionary.learned.iter().map(|l| LearnedView { word: l.word.clone(), heard: l.heard.clone() }).collect(), learn: s.learn.enabled(),`.
In its hand-written `Debug`: `.field("learned", &self.learned.len()).field("learn", &self.learn)`. Run the test: passes.

- [ ] **Step 2: the two pushes in `Ui`** (ipc.rs, next to `tray`):

```rust
    /// The dictionary suggestion, to the Drop (docs/specs/2026-11-02-corrections.md); `None` folds it away.
    pub fn suggest(&self, v: Option<&cue_core::corrections::SuggestView>) {
        self.emit_to(DROP_LABEL, "cue://suggest", v);
    }

    /// The settings after a change the window did not ask for (a word added from the Drop), to every window.
    pub fn settings(&self, v: &SettingsView) {
        self.emit("cue://settings", v);
    }
```

(use the same `emit_to` helper `mic_names` uses; if it is a free function or takes another shape, copy that call).

- [ ] **Step 3: the runtime's state.** In `struct Runtime`, after `tray`:

```rust
    /// The dictionary suggestion from a correction (docs/specs/2026-11-02-corrections.md).
    offers: Offers,
    /// The dictation whose paste is awaited, with its text: the field is watched once it went in.
    watch_next: Option<(DictationId, String)>,
    /// Set to end the field watch early: a new dictation opened the microphone.
    field_stop: Option<Arc<AtomicBool>>,
```

initialized `Offers::default(), None, None` where the struct is built. Add `CoreEvent::Corrected { id: DictationId, pasted: String, span: Option<String> }`
with the doc comment "The field watch after a paste is over: `span` is the corrected text between the paste's anchors.
The text is never logged."

- [ ] **Step 4: start the watch after a dictation's paste.** In `fn insert` add, before `dispatch_insert`:

```rust
        self.watch_next = (self.settings.learn.enabled() && self.attempt.mode != Mode::Ask).then(|| (id, text.clone()));
```

In the `CoreEvent::InsertDone` arm, inside `if self.insert_watch.done(id)`, before `self.feed(…)`:

```rust
                    if outcome == InsertOutcome::Pasted {
                        self.watch_field(id);
                    }
```

```rust
    /// The field just pasted into is read until the user's correction is over (docs/specs/2026-11-02-corrections.md).
    fn watch_field(&mut self, id: DictationId) {
        let Some((wanted, pasted)) = self.watch_next.take() else { return };
        if wanted != id {
            return;
        }
        if let Some(old) = self.field_stop.take() {
            old.store(true, Ordering::Relaxed);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let tx = self.tx.clone();
        let text = pasted.clone();
        if fieldwatch::spawn(text, stop.clone(), move |span| {
            let _ = tx.send(CoreEvent::Corrected { id, pasted, span });
        }) {
            self.field_stop = Some(stop);
        }
    }
```

- [ ] **Step 5: a span becomes an offer; the offer shows at rest.**

```rust
            CoreEvent::Corrected { id: _, pasted, span } => self.on_corrected(&pasted, span.as_deref()),
```

```rust
    fn on_corrected(&mut self, pasted: &str, span: Option<&str>) {
        self.field_stop = None;
        let Some(span) = span else {
            tracing::info!("correction: none");
            return;
        };
        let found = corrections::candidates(pasted, span, &self.settings.dictionary.terms);
        tracing::info!(chars = span.chars().count(), candidates = found.len(), "correction found");
        if !self.settings.learn.enabled() {
            return;
        }
        if let Some(first) = found.into_iter().next() {
            self.offers.offer(first);
            // A missed one it replaced (the dot) is gone from the page until the new one opens at rest.
            self.ui.suggest(self.offers.view().as_ref());
            self.show_offer();
        }
    }

    /// A waiting suggestion opens once the Drop is at rest: idle, no tray card, no answer.
    fn show_offer(&mut self) {
        let rest = self.flow.ui_state().phase == UiPhase::Idle && self.tray.is_none() && !self.flow.answer_open();
        if rest && self.offers.at_rest() {
            self.ui.suggest(self.offers.view().as_ref());
        }
    }
```

Call `self.show_offer()` at the end of `feed` (after `sync_answer()`) and after `self.tray = None` in the `TrayClose`
arm (make that arm a block) and at the end of `on_tray_action`.

- [ ] **Step 6: a dictation drops a shown suggestion and ends the watch.** In `exec`, the `Effect::OpenMic { id }` arm
  becomes:

```rust
            Effect::OpenMic { id } => {
                if let Some(stop) = &self.field_stop {
                    stop.store(true, Ordering::Relaxed);
                }
                if self.offers.dictation_started() {
                    self.ui.suggest(None);
                }
                self.open_mic(id)
            }
```

- [ ] **Step 7: the page's answers.** Before the catch-all `CoreEvent::UiAction(a)` arm:

```rust
            CoreEvent::UiAction(UiAction::SuggestAdd) => {
                if let Some(r) = self.offers.added() {
                    let took = self.learn_word(SettingChange::DictLearn(LearnedWord { word: r.word, heard: r.heard }));
                    if !took {
                        self.offers.undone();
                        self.ui.suggest(None);
                    }
                }
            }
            CoreEvent::UiAction(UiAction::SuggestUndo) => {
                if let Some(r) = self.offers.undone() {
                    self.learn_word(SettingChange::DictRemove(r.word));
                }
                self.ui.suggest(None);
            }
            CoreEvent::UiAction(UiAction::SuggestDismiss) => {
                if self.offers.dismissed() {
                    self.ui.suggest(self.offers.view().as_ref());
                }
            }
            CoreEvent::UiAction(UiAction::SuggestDone) => {
                self.offers.done();
                self.ui.suggest(None);
            }
```

```rust
    /// A change from the Drop's suggestion: applied and written like the window's own, and the windows get the
    /// settings (the main window's Dictionary draws the word). False when the change was refused.
    fn learn_word(&mut self, change: SettingChange) -> bool {
        let took = apply_setting(&mut self.settings, change, &self.paths.settings, self.can_save_settings);
        if took {
            self.ui.settings(&SettingsView::of(&self.settings, self.lang, self.can_save_settings));
        }
        took
    }
```

In `UiReady`, after `self.ui.tray(self.tray);`: `self.ui.suggest(self.offers.view().as_ref());`.

When `Learn(false)` arrives through `change_setting`, end the watch and drop the suggestion: at the end of
`change_setting` add

```rust
        if !self.settings.learn.enabled() {
            if let Some(stop) = self.field_stop.take() {
                stop.store(true, Ordering::Relaxed);
            }
            if self.offers.dictation_started() {
                self.ui.suggest(None);
            }
            self.offers = Offers::default();
        }
```

- [ ] **Step 8: tests.** The runtime has no unit harness for these paths; the logic sits in `Offers` (Task 3) and
  `candidates` (Task 1). Add one ipc test: `Ui`-free, `serde_json::to_value(SettingsView::of(&s, Lang::En, true))`
  with a learned word gives `"learned":[{"word":"Marisol","heard":"Марисоль"}]`. Then `cargo test --workspace`,
  clippy: green; `cargo build -p cue` (app crate) builds.

---

### Task 8: the Drop's capsule and dot

**Files:**
- Modify: `app/ui/src/design/drop.js`, `app/ui/src/design/drop.css`, `app/ui/src/controller.ts`, `app/ui/src/strings.ts`,
  `app/ui/src/main.ts`
- Test: `app/ui/test/controller.test.ts`

**Interfaces:**
- Consumes: `cue://suggest` payload `{ word: string; missed: boolean } | null`; actions `suggestAdd`, `suggestUndo`,
  `suggestDismiss`, `suggestDone` through `io.send`.
- Produces: `DropController.suggest(v: Suggest | null)`, `export interface Suggest { word: string; missed: boolean }`;
  drop.js `Drop.offer(word, missed)`, `Drop.unoffer()`, states `sugg` and `added`, `api.onSuggest(kind)` with
  kind `'add' | 'undo' | 'expire'`.

- [ ] **Step 1: the failing tests** (new `describe` in controller.test.ts, using the file's existing `ctl`, `sent`,
  `rectsSent`):

```ts
describe('Dictionary suggestion', () => {
  const drop = () => document.querySelector('.drop') as HTMLElement;
  const btn = () => drop().querySelector('.sbt') as HTMLButtonElement;
  const dot = () => document.querySelector('.sgdot') as HTMLElement;
  const ring = () => btn().querySelector('.ring circle') as SVGElement;
  const ringOut = () => ring().dispatchEvent(new Event('animationend', { bubbles: true }));

  it('opens the capsule with the word and adds it', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    expect(drop().dataset.s).toBe('sugg');
    expect(drop().querySelector('.swd')!.textContent).toContain('Marisol');
    expect(btn().getAttribute('aria-label')).toBe('Add to dictionary');
    btn().click();
    expect(sent).toEqual(['suggestAdd']);
    expect(drop().dataset.s).toBe('added');
    expect(drop().querySelector('.sbk')!.classList.contains('ok')).toBe(true);
    expect(btn().getAttribute('aria-label')).toBe('Undo');
  });

  it('undo takes the word out; the end of its ring closes', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    btn().click();
    btn().click();
    expect(sent).toEqual(['suggestAdd', 'suggestUndo']);
    ctl.suggest(null); // the core answers an undo with nothing
    expect(drop().dataset.s).toBe('idle');
    ctl.suggest({ word: 'Marisol', missed: false });
    btn().click();
    ringOut();
    expect(sent.at(-1)).toBe('suggestDone');
    expect(drop().dataset.s).toBe('idle');
  });

  it('a ring run out tells the core, and the missed one waits as a dot', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ringOut();
    expect(sent).toEqual(['suggestDismiss']);
    ctl.suggest({ word: 'Marisol', missed: true });
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(true);
  });

  it('hovering a missed one opens the capsule without a ring, leaving folds it back', () => {
    ctl.suggest({ word: 'Marisol', missed: true });
    drop().dispatchEvent(new MouseEvent('mouseenter'));
    expect(drop().dataset.s).toBe('sugg');
    expect(drop().classList.contains('sgwait')).toBe(true);
    drop().dispatchEvent(new MouseEvent('mouseleave'));
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(true);
  });

  it('nothing folds the capsule away; a dictation drops it locally too', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ctl.suggest(null);
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(false);
    ctl.suggest({ word: 'Marisol', missed: true });
    ctl.apply({ phase: 'start', error: null, retry: false, paused: false, mode: 'dictate' });
    expect(dot().classList.contains('on')).toBe(false);
  });

  it('an idle state while the capsule is open keeps it open', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ctl.apply({ phase: 'idle', error: null, retry: false, paused: false, mode: 'dictate' });
    expect(drop().dataset.s).toBe('sugg');
  });

  it('the word goes in as text', () => {
    ctl.suggest({ word: '<b>x</b>', missed: false });
    expect(drop().querySelector('.swd b')).toBeNull();
  });
});
```

Add the Russian names to the strings test if it checks key parity (`controller.test.ts:47` does: new keys go into both).

- [ ] **Step 2: run and see them fail** — from `app`: `npx vitest run test/controller.test.ts`.

- [ ] **Step 3: drop.js.** In the constructor, after the `ok` part:

```js
  P('sbk',`<span class="mi sbk">${IC.book}</span>`);P('swd','<span class="lab swd"></span>');
  P('sbt','<button class="ib solid sbt"></button>');
  this.dot=el('<i class="sgdot" aria-hidden="true"></i>');this.root.append(this.dot);
  this.word='';this.missed=false;this.peek=false;
```

Add `'swd'` to the `TXT` set (the text parts that open to their scrollWidth). In the `if(this.live)` block:

```js
   this.parts.sbt.addEventListener('click',()=>{if(this.s==='sugg'){this.missed=false;this.peek=false;api.onSuggest('add');this.enter('added')}else if(this.s==='added')api.onSuggest('undo')});
   this.parts.sbt.addEventListener('animationend',e=>{if(!e.target.closest||!e.target.closest('.ring'))return;
    if(this.s==='sugg'&&!this.peek){api.onSuggest('expire');this.enter('idle')}else if(this.s==='added'){api.onSuggest('done');this.unoffer()}});
   this.p.addEventListener('mouseenter',()=>{if(this.missed&&this.s==='idle'){this.peek=true;this.enter('sugg')}});
   this.p.addEventListener('mouseleave',()=>{if(this.peek&&this.s==='sugg'){this.peek=false;this.enter('idle')}});
```

(The existing mouseenter/mouseleave that set `hov` stay; both run.) Methods:

```js
 /* The dictionary suggestion (docs/specs/2026-11-02-corrections.md): open — the capsule with its 8 s ring; missed —
    the dot by the idle bar, and hovering opens the capsule without a ring. */
 offer(word,missed){this.word=word;this.missed=!!missed;this.peek=false;
  if(this.missed){if(this.s==='sugg'||this.s==='added')this.enter('idle');else this.layout()}
  else if(this.s==='idle'||this.s==='sugg')this.enter('sugg');else this.layout()}
 unoffer(){this.word='';this.missed=false;this.peek=false;if(this.s==='sugg'||this.s==='added')this.enter('idle');else this.layout()}
```

In `enter(s)`: at the top `if(s==='start'){this.missed=false;this.peek=false}`; for `sugg`/`added` skip the morph's
state change (`this.morph.set('idle',now)`), set the button:

```js
  if(s==='sugg'||s==='added'){const T=this.api.text,add=s==='sugg',t=add?T.learnAdd:T.learnUndo;
   this.parts.sbt.innerHTML=RING+(add?IC.plus:IC.undo);this.parts.sbt.title=t;this.parts.sbt.setAttribute('aria-label',t);
   CUE.setLabel(this.parts.swd,this.word)}
```

(`CUE.setLabel` builds spans from characters, so the word stays text; check `IC.plus` exists in drop.js's `IC`, else
add `plus:SV('<path d="M10 5v10M5 10h10"/>')` in the same style as the other icons.) In `layout()`'s two `V` tables
add `sugg:{sbk:1,swd:1,sbt:1},added:{sbk:1,swd:1,sbt:1}`; at the end of `layout()`:

```js
  this.parts.sbk.classList.toggle('ok',s==='added');
  this.p.classList.toggle('sgwait',s==='sugg'&&this.peek);
  this.dot.classList.toggle('on',s==='idle'&&this.missed);
```

- [ ] **Step 4: drop.css** (after the `.ib` rules; values from the approved mockup, design stage 3):

```css
/* The dictionary suggestion: the Drop's capsule book · word · button, and the dot of a missed one. */
.drop .sbk{transition:color .3s}
.drop .sbk.ok{color:var(--ok)}
.drop[data-s=sugg] .sbt .ring circle{animation:deplete 8s linear forwards}
.drop[data-s=added] .sbt .ring circle{animation:deplete 6s linear forwards}
.drop:hover .sbt .ring circle{animation-play-state:paused}
.drop.sgwait .sbt .ring{display:none}
.sgdot{position:absolute;left:calc(100% + 5px);bottom:1px;width:5px;height:5px;border-radius:50%;background:var(--ok);
  box-shadow:0 0 0 1.5px rgba(18,18,20,.9);opacity:0;transform:scale(0);transition:transform .4s var(--e-spring),opacity .2s}
.sgdot.on{opacity:1;transform:scale(1)}
@media (prefers-reduced-motion:reduce){.drop .sbt .ring circle{animation-duration:.01s}.sgdot{transition:none}}
```

Check where `.pv` is positioned so `.sgdot` sits by the idle bar (it is appended to `this.root`, the `.pv`); if the
reduced-motion rule would make the 8 s ring end at once, drop that line instead and leave the ring as it is (the
existing cancel ring is the model to follow: copy what it does under reduced motion).

- [ ] **Step 5: strings.ts.** In `DropText` add `learnAdd: string; learnUndo: string;` with doc comments; `en`:
  `learnAdd: 'Add to dictionary', learnUndo: 'Undo'`; `ru`: `learnAdd: 'Добавить в словарь', learnUndo: 'Отменить'`.

- [ ] **Step 6: controller.ts.**

```ts
/** The dictionary suggestion from the core (cue://suggest): the word, and whether it was missed (the dot). */
export interface Suggest { word: string; missed: boolean }
```

`UiAction` gains `| 'suggestAdd' | 'suggestUndo' | 'suggestDismiss' | 'suggestDone'`. In the Drop `api` object:

```ts
      onSuggest: (k: 'add' | 'undo' | 'expire' | 'done') => {
        this.io.send(k === 'add' ? 'suggestAdd' : k === 'undo' ? 'suggestUndo' : k === 'expire' ? 'suggestDismiss' : 'suggestDone');
        this.wake();
      },
```

```ts
  /** The core's suggestion: shown only on the idle Drop (the core sends it at rest); null folds it away. */
  suggest(v: Suggest | null) {
    if (v) this.drop.offer(v.word, v.missed);
    else this.drop.unoffer();
    this.wake();
  }
```

In `apply(s)`, where it calls `this.drop.enter(...)`: an idle state must not fold an open capsule —
`const keep = s.phase === 'idle' && (this.drop.s === 'sugg' || this.drop.s === 'added');` and skip the `enter` when
`keep` (call `this.drop.layout()` instead). In `frame()`'s `calm`: add `&& this.drop.s !== 'sugg' && this.drop.s !== 'added'`
so the rectangles follow the capsule while it grows.

- [ ] **Step 7: main.ts.** Next to the `cue://tray` listener:
  `await listen<Suggest | null>('cue://suggest', (e) => ctl.suggest(e.payload));` (same shape as the others there).

- [ ] **Step 8: run** `npx vitest run` and `npx tsc --noEmit -p tsconfig.json` from `app`: green.

---

### Task 9: the main window — green book, hint, the switch

**Files:**
- Modify: `app/ui/src/window/api.ts`, `page.ts`, `dictionary.ts`, `settings.ts`, `text.ts`, `app/ui/src/design/window.css`
- Test: `app/ui/test/windowFixture.ts`, `app/ui/test/window-dictionary.test.ts`, `app/ui/test/window-settings.test.ts`

**Interfaces:**
- Consumes: `SettingsView.learned: { word: string; heard: string }[]`, `SettingsView.learn: boolean`, `cue://settings`.
- Produces: `api.onSettings(cb)`; `SettingChange` gains `{ kind: 'learn'; value: boolean }`.

- [ ] **Step 1: the failing tests.** In `windowFixture.ts`'s `view()` add `learned: []` and `learn: true`; in its
  `stub()`, simulate `dictRemove` removing from `learned` too. In `window-dictionary.test.ts`:

```ts
  it('a word from a correction has the green book and says how it was heard', async () => {
    const w = await mountWith({ dictionary: ['Marisol', 'Soniox'], learned: [{ word: 'Marisol', heard: 'Марисоль' }] });
    const chip = w.querySelector('.dch[data-word="Marisol"]') as HTMLElement;
    expect(chip.querySelector('.g svg')).not.toBeNull();
    expect(chip.title).toBe('sounds like «Марисоль»');
    expect((w.querySelector('.dch[data-word="Soniox"]') as HTMLElement).querySelector('.g')).toBeNull();
  });

  it('the hint tells what the green book means', async () => {
    const w = await mountWith({});
    const hint = w.querySelector('.dhint') as HTMLElement;
    expect(hint.textContent).toBe('Words CUE must write right.  — from your corrections: CUE noticed the change and offered the word.');
    expect(hint.querySelector('svg')).not.toBeNull();
  });

  it('settings pushed by the core redraw the dictionary', async () => {
    const w = await mountWith({});
    pushSettings({ dictionary: ['Marisol'], learned: [{ word: 'Marisol', heard: 'Марисоль' }] });
    expect(w.querySelector('.dch[data-word="Marisol"] .g')).not.toBeNull();
  });
```

(`mountWith` and `pushSettings` stand for the file's own helpers: use the existing mount helper with a view override;
for `pushSettings`, make the fixture's api stub keep the `onSettings` callback and expose a function that calls it.
Check the exact text the hint test expects against what `h()` and `icon()` produce and adjust the spaces in the
expectation only.) In `window-settings.test.ts`, next to "autostart switch calls autostart_set":

```ts
  it('"Learn from me" switch sends the learn change', async () => {
    const tray = await openGeneral();
    const sw = tray.querySelector('[data-sw="learn"]') as HTMLButtonElement;
    expect(sw.getAttribute('aria-checked')).toBe('true');
    sw.click();
    await flush();
    expect(calls).toContainEqual(['settings_set', { change: { kind: 'learn', value: false } }]);
    expect(sw.getAttribute('aria-checked')).toBe('false');
  });
```

(again with the file's own helpers for opening the General page, the recorded calls and flushing promises).

- [ ] **Step 2: run and see them fail** — `npx vitest run test/window-dictionary.test.ts test/window-settings.test.ts`.

- [ ] **Step 3: implement.**
  - `api.ts`: in `SettingsView` add `/** Words added from a correction, with how they were heard. */ learned: { word: string; heard: string }[];`
    and `/** "Learn from me". */ learn: boolean;`; in `SettingChange` add `| { kind: 'learn'; value: boolean }`; add

```ts
/** Settings the core changed by itself (a word added from the Drop's suggestion). */
export async function onSettings(cb: (v: SettingsView) => void): Promise<void> {
  await getCurrentWebviewWindow().listen<SettingsView>('cue://settings', (e) => cb(e.payload));
}
```

  - `page.ts`: next to `api.onModel(...)`: `api.onSettings((v) => this.took(v)).catch((e) => logFailed('following the settings', e));`
    (`took` is the method that already takes a view after `settingsSet`; if it is private to another object, call
    the same thing `change()` calls with the view).
  - `text.ts`: `dictHintLearned: string` (doc: "The hint's second half, after the green book.") — en
    `'— from your corrections: CUE noticed the change and offered the word.'`, ru
    `'— из ваших правок: CUE заметил замену и предложил слово.'`; `dictHeard: string` (doc: "A learned chip's tooltip;
    {HEARD} is how it was heard.") — en `'sounds like «{HEARD}»'`, ru `'звучит как «{HEARD}»'`; `learn` — en
    `'Learn from me'`, ru `'Учиться у меня'`; `learnHint` — en `'CUE notices your corrections and offers words for the dictionary. Everything stays on this computer.'`,
    ru `'CUE замечает ваши правки и предлагает слова для словаря. Всё остаётся на этом компьютере.'`.
  - `dictionary.ts` `mount`: the hint becomes

```ts
    const hint = h('p', 'dhint', t.dictHint + ' ');
    hint.insertAdjacentHTML('beforeend', icon('book'));
    hint.append(' ' + t.dictHintLearned);
```

    and in `render()`, before `chip.append(w)`:

```ts
        const learned = this.ctx.settings?.learned?.find((l) => l.word === w);
        if (learned) {
          const g = h('i', 'g');
          g.innerHTML = icon('book');
          chip.append(g);
          chip.title = fill(t.dictHeard, learned.heard);
        }
```

    (check `fill`'s placeholder convention in text.ts — `{NAME}` style — and use `{HEARD}` the same way; if `fill`
    takes one value positionally, it fills `{HEARD}`.)
  - `window.css` next to `.dhint`: `.dhint svg{display:inline-block;width:13px;height:13px;vertical-align:-2px;color:var(--ok)}`
    (it exists: check, keep one) and `.dch .g{display:grid;color:var(--ok)}.dch .g svg{width:13px;height:13px}`.
  - `settings.ts` `generalRows()`: add a second switch built like `autostart`, `sw.dataset.sw = 'learn'`,
    `aria-checked` from `this.ctx.settings?.learn ?? true`, label `t.learn`, row `srow(t.learn, t.learnHint, sw).row`.
    In `onClick`, next to the autostart branch:

```ts
    else if (t.closest('[data-sw="learn"]')) this.ctx.change({ kind: 'learn', value: !(this.ctx.settings?.learn ?? true) });
```

    and in `settingsChanged()` (called after every change) set the switch's `aria-checked` from the new view.

- [ ] **Step 4: run** `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`: green.

---

### Task 10: documents, then the live check

**Files:**
- Modify: `README.md`, `docs/ARCHITECTURE.md`

- [ ] **Step 1: README.** In the privacy part, one paragraph: with "Learn from me" on, after a paste CUE reads the
  field it pasted into for up to a minute to notice a corrected word; the text stays in memory, nothing of it is
  written or sent; the word goes into the dictionary only when you press "+". Where the settings are listed, the
  switch on the General page.
- [ ] **Step 2: ARCHITECTURE.** The flow from paste to suggestion: `insert` → `InsertDone` → `fieldwatch::spawn`
  (`cue-field` thread, UI Automation) → `CoreEvent::Corrected` → `corrections::candidates` → `Offers` →
  `cue://suggest` → the Drop's capsule → `suggestAdd` → `SettingChange::DictLearn` → `cue://settings`; and that
  `fieldwatch`'s `FieldReader` is where another system plugs in its own reader.
- [ ] **Step 3: all checks** from the global constraints: green. `cargo run -p cue-cli -- leak-scan` (or the command
  `scripts/build-release.ps1 -ScanBuild` uses) still passes on a settings.json with a learned word.
- [ ] **Step 4: live, with the owner** (build the app the usual way, `npx tauri dev` from `app` or the release build):
  1. Claude (desktop): dictate "Мне нужно позвонить Марисоль", correct to "Marisol": the capsule comes ~2.5 s after the
     last keystroke; "+" adds it; the Dictionary shows it with the green book and the tooltip.
  2. Brave's address bar: the same with another name.
  3. Let the 8 s ring run out: the dot; hover the Drop: the capsule without a ring; start a dictation: the dot goes.
  4. "Undo" after "+": the word leaves the Dictionary.
  5. "Learn from me" off: no capsule after a correction.
  6. Telegram and a field inside a web page: note whether a capsule comes (the spec expects nothing where the field
     cannot be read; record what happens for the owner).
  7. Both themes.
- [ ] **Step 5: report to the owner** what passed, what did not, and ask whether to commit.

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


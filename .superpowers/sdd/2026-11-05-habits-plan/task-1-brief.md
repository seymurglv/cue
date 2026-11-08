### Task 1: kinds, forms and their hash (`cue-core::habits`, pure)

**Files:**
- Modify: `app/crates/core/src/corrections.rs` (the alignment loop of `changes`, line 51, becomes `runs`)
- Create: `app/crates/core/src/habits.rs`
- Modify: `app/crates/core/src/lib.rs` (add `pub mod habits;` after `pub mod flow;`)

**Interfaces:**
- Produces in `corrections` (crate-visible): `pub(crate) const MAX_RUN`, `pub(crate) fn key(&str) -> String`,
  `pub(crate) fn clean(&str) -> &str`, `pub(crate) fn looks_like_name(&str, bool, &str) -> bool`,
  `pub(crate) fn words(&str) -> Vec<&str>`, `pub(crate) struct Run { pub old: Range<usize>, pub new: Range<usize> }`,
  `pub(crate) fn runs(a: &[&str], b: &[&str]) -> Vec<Run>`, `pub(crate) fn sentence_start(b: &[&str], j: usize) -> bool`.
- Produces in `habits`: `pub enum Form { EndMark { mark: char, removed: bool }, FirstLetter { lower: bool },
  Replacement { from: String, to: String }, Digits, Dash }` (Clone, PartialEq, Eq, Serialize, Deserialize, JSON
  `{"kind":"endMark","mark":".","removed":true}`; hand-written `Debug`) with `pub fn kind(&self) -> &'static str`,
  `pub fn hash(&self) -> String` (16 hex digits), `pub fn shows_in(&self, text: &str) -> bool`;
  `pub struct Found { pub form: Form, .. }`; `pub struct Correction { pub found: Vec<Found>, .. }`;
  `pub fn correction(pasted: &str, corrected: &str) -> Correction`; constants `MAX_EXAMPLE_CHARS = 40`.

- [ ] **Step 1: write the failing tests** at the end of the new `habits.rs` (the module's top comes in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(pasted: &str, corrected: &str) -> Vec<Form> {
        correction(pasted, corrected).found.into_iter().map(|f| f.form).collect()
    }
    fn example(pasted: &str, corrected: &str) -> (String, String) {
        let c = correction(pasted, corrected);
        let all: Vec<&Found> = c.found.iter().collect();
        c.example(&all)
    }
    const STOP: Form = Form::EndMark { mark: '.', removed: true };

    #[test]
    fn the_end_mark_and_the_first_letter_in_one_correction() {
        assert_eq!(kinds("Скинь файл.", "скинь файл"), [Form::FirstLetter { lower: true }, STOP]);
    }

    #[test]
    fn an_end_mark_added_or_removed_but_not_swapped() {
        assert_eq!(kinds("привет мир", "привет, мир!"), [Form::EndMark { mark: '!', removed: false }]);
        assert_eq!(kinds("Посмотрю вечером…", "Посмотрю вечером"), [Form::EndMark { mark: '…', removed: true }]);
        assert!(kinds("Готово!", "Готово?").is_empty(), "one mark for another is neither");
    }

    #[test]
    fn a_capital_given_is_a_first_letter_too() {
        assert_eq!(kinds("скинь файл", "Скинь файл"), [Form::FirstLetter { lower: false }]);
    }

    #[test]
    fn a_replacement_of_ordinary_words() {
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert_eq!(kinds("Здравствуйте, Анна!", "Добрый день, Анна!"), [hello]);
    }

    #[test]
    fn names_are_left_to_the_dictionary() {
        assert!(kinds("Мне нужно позвонить Марисоль.", "Мне нужно позвонить Marisol.").is_empty());
        assert!(kinds("открой гитхаб сейчас", "открой GitHub сейчас").is_empty());
    }

    #[test]
    fn digits_for_number_words() {
        assert_eq!(kinds("рост двенадцать процентов за год", "рост 12 % за год"), [Form::Digits]);
    }

    #[test]
    fn a_dash_for_a_hyphen_once_per_correction() {
        assert_eq!(kinds("клиенты - в основном малый бизнес", "клиенты — в основном малый бизнес"), [Form::Dash]);
        assert_eq!(kinds("ну - да - конечно", "ну — да — конечно"), [Form::Dash], "a form counts once");
    }

    #[test]
    fn case_or_punctuation_inside_the_text_is_no_habit() {
        assert!(kinds("позвони github завтра", "позвони GitHub завтра").is_empty(), "case only");
        assert!(kinds("да нет наверное", "да, нет, наверное").is_empty(), "commas only");
    }

    #[test]
    fn a_rewritten_text_holds_nothing() {
        assert!(kinds("Скинь файл.", "Позвоню позже").is_empty());
    }

    /// FNV-1a over the kind and its detail: the same in every version, so a count survives an update.
    #[test]
    fn the_hash_is_stable() {
        assert_eq!(Form::Digits.hash(), "620301a9632d813f");
        assert_eq!(Form::Dash.hash(), "85657e6730abe435");
        assert_eq!(STOP.hash(), "94128fbe83ca55c4");
        assert_eq!(Form::FirstLetter { lower: true }.hash(), "3e6b38c8e27e485e");
        assert_eq!(Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() }.hash(), "84a975dc80118b92");
        assert_ne!(Form::EndMark { mark: '.', removed: false }.hash(), STOP.hash());
    }

    #[test]
    fn the_example_is_the_corrected_words_with_two_around() {
        assert_eq!(example("Скинь файл.", "скинь файл"), ("Скинь файл.".into(), "скинь файл".into()));
        assert_eq!(
            example("Мы выросли на двенадцать процентов за этот год", "Мы выросли на 12 % за этот год"),
            ("выросли на двенадцать процентов за этот".into(), "выросли на 12 % за этот".into()),
        );
    }

    #[test]
    fn an_example_side_is_never_longer_than_forty_characters() {
        let pasted = "Пожалуйста посмотрите приложенный квартальный отчёт до вечера пятницы.";
        let corrected = "Пожалуйста посмотрите приложенный квартальный отчёт до вечера пятницы";
        assert_eq!(example(pasted, corrected), ("до вечера пятницы.".into(), "до вечера пятницы".into()));
        let one = "Превысокомногорассмотрительствующий-предлинный-пример.";
        let (before, after) = example(one, one.trim_end_matches('.'));
        assert_eq!((before.chars().count(), after.chars().count()), (MAX_EXAMPLE_CHARS, MAX_EXAMPLE_CHARS));
        assert!(before.starts_with('…') && before.ends_with("пример."), "an end mark keeps the text's end: {before}");
    }

    #[test]
    fn debug_shows_no_words() {
        let c = correction("Здравствуйте, Анна!", "Добрый день, Анна!");
        let shown = format!("{c:?} {:?} {:?}", c.found[0], c.found[0].form);
        for word in ["Анна", "добрый", "здравствуйте"] {
            assert!(!shown.contains(word), "{word} in {shown}");
        }
    }
}
```

- [ ] **Step 2: run them and see them fail**

Run: `cargo test -p cue-core habits` (from `C:\Users\seymurglv\Desktop\cue-dev`). Expected: compile errors, `correction`
and `Form` are not defined.

- [ ] **Step 3: share the alignment in `corrections.rs`.** Make the helpers crate-visible: `const MAX_RUN` →
  `pub(crate) const MAX_RUN`, `fn clean` → `pub(crate) fn clean`, `fn key` → `pub(crate) fn key`,
  `fn looks_like_name` → `pub(crate) fn looks_like_name`; add `use std::ops::Range;` at the top. Replace `changes`
  (line 51 to the end of the function) with the aligned runs and the dictionary's reading of them (the same behavior:
  every existing `corrections` test still passes):

```rust
/// The words of a text as the diff splits them: by white space.
pub(crate) fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// One replaced run: the words `old` of the pasted text gave way to the words `new` of the corrected one; neither is
/// empty.
pub(crate) struct Run {
    pub old: Range<usize>,
    pub new: Range<usize>,
}

/// A word at `j` of `b` starts a sentence: the first one, or one after `.`, `!`, `?` or `…` (closing quotes and a
/// bracket aside).
pub(crate) fn sentence_start(b: &[&str], j: usize) -> bool {
    j == 0 || b[j - 1].trim_end_matches(['"', '»', '”', ')']).ends_with(SENTENCE_END)
}

/// The replaced runs of any length, aligned word by word (the longest common subsequence of the words compared without
/// case and outer punctuation). Deleted and inserted words are not runs; texts of more than `MAX_TOKENS` words have
/// none.
pub(crate) fn runs(a: &[&str], b: &[&str]) -> Vec<Run> {
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
            if gi < i && gj < j {
                out.push(Run { old: gi..i, new: gj..j });
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

/// The replaced runs of one to three words on each side (see [`runs`]); of a run with more new words than old ones,
/// the new side keeps only its leading names and terms.
fn changes(pasted: &str, corrected: &str) -> Vec<Change> {
    let (a, b) = (words(pasted), words(corrected));
    let mut out = Vec::new();
    for r in runs(&a, &b) {
        let (old, mut new) = (&a[r.old], &b[r.new.clone()]);
        // More words written than were heard: only the leading ones that are each a name or a term belong to the
        // word, the rest is text typed on after it ("Марго и скажи" for "марине" is "Марго").
        if new.len() > old.len() {
            let named = new
                .iter()
                .enumerate()
                .take_while(|&(k, w)| looks_like_name(clean(w), sentence_start(&b, r.new.start + k), corrected));
            new = &new[..named.count()];
        }
        if !new.is_empty() && old.len() <= MAX_RUN && new.len() <= MAX_RUN {
            out.push(Change {
                heard: clean(&old.join(" ")).to_owned(),
                word: clean(&new.join(" ")).to_owned(),
                sentence_start: sentence_start(&b, r.new.start),
            });
        }
    }
    out
}
```

- [ ] **Step 4: the module's top, the forms and their detection** (`habits.rs`, above `mod tests`):

```rust
//! Habits (docs/specs/2026-11-04-habits.md): the same correction made again and again after a dictation becomes a rule
//! the cleanup follows. Plain code over strings: which kinds of correction a pasted text and its corrected span hold
//! (`correction`), how they are counted and learned (`Habits`), the rule's request and its template, and whether a
//! cleaned text shows a habit's effect (`marks`). The runtime watches the field and asks the cleanup's connection.
//!
//! A form is counted by its hash, never by its text. A habit keeps its example, at most 40 characters on each side: the
//! only text of a correction ever written to disk. `Debug` everywhere shows lengths.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::corrections;
use crate::refine;

/// The longest side of an example, in characters.
pub const MAX_EXAMPLE_CHARS: usize = 40;
/// Words of context kept around the corrected ones on each side of an example.
const CONTEXT_WORDS: usize = 2;
/// The sentence marks an end mark is made of.
const END_MARKS: [char; 4] = ['.', '!', '?', '…'];

/// A kind of correction with its detail; counted by its [`Form::hash`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Form {
    /// The last sentence mark removed (`removed`) or added.
    EndMark { mark: char, removed: bool },
    /// The first letter's case changed: to lower case (`lower`), or to a capital.
    FirstLetter { lower: bool },
    /// A run of one to three words replaced by another, both in lower case without outer punctuation: words of the
    /// habit's example.
    Replacement { from: String, to: String },
    /// Number words replaced by digits.
    Digits,
    /// A hyphen between words replaced by a dash.
    Dash,
}

/// A replacement's words may be the user's: only their lengths.
impl std::fmt::Debug for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EndMark { mark, removed } => f.debug_struct("EndMark").field("mark", mark).field("removed", removed).finish(),
            Self::FirstLetter { lower } => f.debug_struct("FirstLetter").field("lower", lower).finish(),
            Self::Replacement { from, to } => f
                .debug_struct("Replacement")
                .field("from_chars", &from.chars().count())
                .field("to_chars", &to.chars().count())
                .finish(),
            Self::Digits => f.write_str("Digits"),
            Self::Dash => f.write_str("Dash"),
        }
    }
}

/// FNV-1a, 64 bits: a hash that never changes between versions (std's `DefaultHasher` may).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

impl Form {
    /// The kind's name, as the log and the rule request say it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::EndMark { .. } => "endMark",
            Self::FirstLetter { .. } => "firstLetter",
            Self::Replacement { .. } => "replacement",
            Self::Digits => "digits",
            Self::Dash => "dash",
        }
    }

    /// The order a habit lists its forms in, and its rule reads them: first letter, end mark, then the rest.
    fn rank(&self) -> u8 {
        match self {
            Self::FirstLetter { .. } => 0,
            Self::EndMark { .. } => 1,
            Self::Replacement { .. } => 2,
            Self::Digits => 3,
            Self::Dash => 4,
        }
    }

    /// What the form is counted by: the hash of its kind and detail, 16 hex digits. Never its text.
    pub fn hash(&self) -> String {
        let canon = match self {
            Self::EndMark { mark, removed } => format!("endMark:{}{mark}", if *removed { '-' } else { '+' }),
            Self::FirstLetter { lower } => format!("firstLetter:{}", if *lower { "lower" } else { "upper" }),
            Self::Replacement { from, to } => format!("replacement:{from}\u{1f}{to}"),
            Self::Digits => "digits".to_owned(),
            Self::Dash => "dash".to_owned(),
        };
        format!("{:016x}", fnv1a(canon.as_bytes()))
    }

    /// Whether `text`, as the cleanup wrote it, shows this form: the end mark absent (or there), the first letter in
    /// its case, the replacement's new words there and its old ones not, digits, a dash and no spaced hyphen.
    pub fn shows_in(&self, text: &str) -> bool {
        let t = text.trim();
        match self {
            Self::EndMark { mark, removed } => t.ends_with(*mark) != *removed,
            Self::FirstLetter { lower } => t.chars().find(|c| c.is_alphabetic()).is_some_and(|c| c.is_lowercase() == *lower),
            Self::Replacement { from, to } => {
                let keys: Vec<String> = t.split_whitespace().map(corrections::key).collect();
                has_run(&keys, to) && !has_run(&keys, from)
            }
            Self::Digits => t.chars().any(|c| c.is_ascii_digit()),
            Self::Dash => (t.contains(" — ") || t.contains(" – ")) && !t.contains(" - "),
        }
    }
}

/// `words` (compared as the diff compares them) stand in a row in `keys`.
fn has_run(keys: &[String], words: &str) -> bool {
    let run: Vec<String> = words.split_whitespace().map(corrections::key).collect();
    !run.is_empty() && keys.windows(run.len()).any(|w| w == run.as_slice())
}

/// A form found in one correction, and where: its words in the pasted text (`old`) and in the corrected one (`new`).
#[derive(Clone, PartialEq, Eq)]
pub struct Found {
    pub form: Form,
    old: Range<usize>,
    new: Range<usize>,
}

impl std::fmt::Debug for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Found").field("form", &self.form).field("old", &self.old).field("new", &self.new).finish()
    }
}

/// A pasted text and its corrected span, as words, with the forms they hold. It lives in memory only.
pub struct Correction {
    old: Vec<String>,
    new: Vec<String>,
    pub found: Vec<Found>,
}

impl std::fmt::Debug for Correction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Correction")
            .field("old_words", &self.old.len())
            .field("new_words", &self.new.len())
            .field("found", &self.found)
            .finish()
    }
}

/// The kinds of correction between what CUE pasted and the corrected span, each form once, in the order first letter,
/// end mark, then the replaced runs as they stand. A text that shares no word with the paste (rewritten, another
/// message) holds none; names and terms are the dictionary's (`corrections::candidates`), never a replacement; a change
/// of case or punctuation inside the text is none of these.
pub fn correction(pasted: &str, corrected: &str) -> Correction {
    let (a, b) = (corrections::words(pasted), corrections::words(corrected));
    let mut found = Vec::new();
    let kb: Vec<String> = b.iter().map(|w| corrections::key(w)).collect();
    let shared = a.iter().map(|w| corrections::key(w)).any(|k| !k.is_empty() && kb.contains(&k));
    if shared && a.len() <= corrections::MAX_TOKENS && b.len() <= corrections::MAX_TOKENS {
        first_letter(&a, &b, &mut found);
        end_mark(&a, &b, &mut found);
        for run in corrections::runs(&a, &b) {
            replaced(&a, &b, run, corrected, &mut found);
        }
    }
    let mut hashes = Vec::new();
    found.retain(|f| {
        let h = f.form.hash();
        let first = !hashes.contains(&h);
        hashes.push(h);
        first
    });
    let own = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect();
    Correction { old: own(&a), new: own(&b), found }
}

/// The same first word, its first letter in another case.
fn first_letter(a: &[&str], b: &[&str], out: &mut Vec<Found>) {
    let (Some(&x), Some(&y)) = (a.first(), b.first()) else { return };
    let letter = |w: &str| w.chars().find(|c| c.is_alphabetic());
    if corrections::key(x) != corrections::key(y) {
        return;
    }
    if let (Some(p), Some(q)) = (letter(x), letter(y))
        && p != q
        && p.to_lowercase().eq(q.to_lowercase())
    {
        out.push(Found { form: Form::FirstLetter { lower: q.is_lowercase() }, old: 0..1, new: 0..1 });
    }
}

/// The same last word, its sentence mark removed or added (one mark for another is neither).
fn end_mark(a: &[&str], b: &[&str], out: &mut Vec<Found>) {
    let (Some(&x), Some(&y)) = (a.last(), b.last()) else { return };
    if corrections::key(x) != corrections::key(y) {
        return;
    }
    let mark = |w: &str| w.chars().last().filter(|c| END_MARKS.contains(c));
    let form = match (mark(x), mark(y)) {
        (Some(m), None) => Form::EndMark { mark: m, removed: true },
        (None, Some(m)) => Form::EndMark { mark: m, removed: false },
        _ => return,
    };
    let (i, j) = (a.len() - 1, b.len() - 1);
    out.push(Found { form, old: i..i + 1, new: j..j + 1 });
}

/// A replaced run: a dash for a spaced hyphen, digits for number words, or ordinary words (one to three on each side,
/// each side at most `MAX_EXAMPLE_CHARS`, not a name or a term).
fn replaced(a: &[&str], b: &[&str], run: corrections::Run, corrected: &str, out: &mut Vec<Found>) {
    let (old, new) = (&a[run.old.clone()], &b[run.new.clone()]);
    let (from, to) = (old.join(" "), new.join(" "));
    let digit = |s: &str| s.chars().any(|c| c.is_ascii_digit());
    let form = if old == ["-"] && matches!(new, ["—"] | ["–"]) {
        Form::Dash
    } else if digit(&to) && !digit(&from) && from.chars().any(char::is_alphabetic) {
        Form::Digits
    } else {
        let name = corrections::looks_like_name(
            corrections::clean(&to),
            corrections::sentence_start(b, run.new.start),
            corrected,
        );
        let (from, to) = (corrections::clean(&from).to_lowercase(), corrections::clean(&to).to_lowercase());
        let fits = |s: &str| s.chars().count() <= MAX_EXAMPLE_CHARS && refine::is_term(s);
        if name || old.len() > corrections::MAX_RUN || new.len() > corrections::MAX_RUN || !fits(&from) || !fits(&to) || from == to {
            return;
        }
        Form::Replacement { from, to }
    };
    out.push(Found { form, old: run.old, new: run.new });
}

/// `s` cut to `max` characters with "…", keeping its end (`tail`) or its start.
fn cut(s: &str, max: usize, tail: bool) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_owned();
    }
    if tail {
        format!("…{}", s.chars().skip(n - (max - 1)).collect::<String>())
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

impl Correction {
    /// The example of these forms: their words with up to two words around them on each side, at most
    /// `MAX_EXAMPLE_CHARS` on each side, fewer words around when it is longer. When even their own words do not fit,
    /// a replacement's own words (they must stay in the example), else the last form's; a side still too long is cut,
    /// keeping the text's end for an end mark and its start otherwise.
    fn example(&self, found: &[&Found]) -> (String, String) {
        let span = |f: &[&Found]| {
            let lo = |r: fn(&Found) -> usize| f.iter().map(|x| r(x)).min().unwrap_or(0);
            let hi = |r: fn(&Found) -> usize| f.iter().map(|x| r(x)).max().unwrap_or(0);
            (lo(|x| x.old.start), hi(|x| x.old.end), lo(|x| x.new.start), hi(|x| x.new.end))
        };
        let side = |w: &[String], lo: usize, hi: usize, ctx: usize| w[lo.saturating_sub(ctx)..(hi + ctx).min(w.len())].join(" ");
        let fit = |f: &[&Found]| {
            let (a0, a1, b0, b1) = span(f);
            (0..=CONTEXT_WORDS)
                .rev()
                .map(|c| (side(&self.old, a0, a1, c), side(&self.new, b0, b1, c)))
                .find(|(x, y)| x.chars().count() <= MAX_EXAMPLE_CHARS && y.chars().count() <= MAX_EXAMPLE_CHARS)
        };
        if let Some(e) = fit(found) {
            return e;
        }
        let replacement: Vec<&Found> =
            found.iter().copied().filter(|f| matches!(f.form, Form::Replacement { .. })).take(1).collect();
        if !replacement.is_empty()
            && let Some(e) = fit(&replacement)
        {
            return e;
        }
        let last = &found[found.len() - 1..];
        let (a0, a1, b0, b1) = span(last);
        let tail = matches!(last[0].form, Form::EndMark { .. });
        let side0 = |w: &[String], lo, hi| cut(&side(w, lo, hi, 0), MAX_EXAMPLE_CHARS, tail);
        (side0(&self.old, a0, a1), side0(&self.new, b0, b1))
    }
}
```

  Task 2 widens the imports to `use serde::{Deserialize, Deserializer, Serialize};` and adds
  `use crate::history::{self, HabitMark};`.

- [ ] **Step 5: run the tests and see them pass.** Run: `cargo test -p cue-core` (the `corrections` tests included:
  the refactor keeps their behavior). Byte slicing is never used: words are slices of `split_whitespace`, `cut` counts
  characters.

- [ ] **Step 6: clippy** — `cargo clippy -p cue-core --all-targets -- -D warnings`, clean.

---


## Coordinator rulings for this task (binding, override the text above)
- Step 6: run clippy for the core crate only after Task 2 makes the helpers used; in this task run `cargo test -p cue-core` and `cargo build`; dead-code warnings for `rank`, `example`, `cut`, `CONTEXT_WORDS` are expected until Task 2 (do not silence them with allow attributes).

# Habits: implementation plan

**Goal:** when the user corrects the same thing after a dictation again and again in one app, CUE learns it as a habit,
says so once with a tray card ("New habit", Undo), and from then on the cleanup writes it the user's way; the main
window gets a Habits section, and the History shows which habits shaped an entry.

**Architecture:** Everything that can be plain code over strings lives in `cue-core::habits`: the kinds of correction
found in a pasted text and its corrected span (the correction watch of parts 3 and 4), their forms and hashes, the
example, counting, learning, widening and declining, the template rules, the rule request's messages and the answer's
check, and whether a cleaned text shows a habit (`marks`). `cue-core::refine` writes the habits block of the cleanup
request; `cue-engines::habit` asks the cleanup's connection for a rule. The runtime counts each dictation's correction,
asks for the rule (5 s, else the template), saves the habit, shows its card when the Drop is at rest (one notice at a
time with the dictionary suggestion), sends the habits with a dictation's cleanup and records the applied ones on the
history entry. The Drop page draws the card (`newHabit`); the main window draws the Habits section and the History's
chips.

**Tech stack:** Rust 2024 (workspace `app/`), Tauri 2, TypeScript + vitest (jsdom).

**Spec:** `docs/specs/2026-11-04-habits.md` (binding). Read it first; this plan argues from it. It builds on
`docs/specs/2026-11-02-corrections.md` and its plan `docs/specs/2026-11-03-corrections-plan.md`.

## Global constraints

- Files are in English and say nothing about the tools the code was written with.
- No commits unless the owner asks; each task ends with the checks green, not with a commit.
- The field's text never reaches disk or the log: the log gets "correction found / none", "habit forms counted",
  "habit learned" with the kinds and numbers only. Every new `Debug` that could hold text prints its length (see
  `SettingChange`'s hand-written `Debug`, `LearnedWord`'s): `Form`, `Found`, `Correction`, `Seen`, `Habit`, `Draft`,
  `RuleAsk`, `RuleText`, `HabitMark`, `RefineContext`.
- The habit's example (`before`/`after`, at most 40 characters each) is the only text of a correction ever written to
  disk (decided 04.11.2026). A replacement's words, kept in its form, are always words of that example (Task 1 makes
  sure of it). The rule and its short name are written from it.
- `learn.enabled` off: no field is read, nothing is counted, learned or applied; no habit goes into a request; habits
  waiting for their rule or their card are dropped.
- Texts in English and Russian; the card and the section's buttons have accessible names.
- Motion follows the family: a new row grows in (`height .45s var(--e-sm)`) with `rowIn .6s var(--e-spring)` and the
  green highlight of a new dictionary chip; a removed one folds away the same way; switching a habit off mutes its row in
  place (color, never opacity: `--muted` keeps contrast ≥ 4.5:1); labels change with `CUE.setLabel` (shared letters
  stay); the card grows out of the Drop like the other tray cards; reduced motion is respected (rows appear and go at
  once).
- Checks, from `C:\Users\seymurglv\Desktop\cue-dev`: `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`; from `app`: `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`.

## File map

| File | Change |
|---|---|
| `app/crates/core/src/corrections.rs` | the aligned runs (`words`, `Run`, `runs`, `sentence_start`) shared with habits; `key`, `clean`, `looks_like_name`, `MAX_RUN` crate-visible; `Offers::waiting`, `Offers::open` |
| `app/crates/core/src/habits.rs` | new: `Form`, `Found`, `Correction`, `correction`, example, `Seen`, `Habit`, `Habits` (count, adopt, forget, switch, for_app), `Draft`, `Counted`, `RuleAsk`, `RuleText`, `template`, `unscoped`, `rule_messages`, `read_rule`, `marks` |
| `app/crates/core/src/lib.rs` | `pub mod habits;` |
| `app/crates/core/src/history.rs` | `HabitMark`, `Entry.habits` |
| `app/crates/core/src/settings.rs` | `Settings.habits`, `SettingChange::{HabitOn, HabitForget, HabitsForget}` |
| `app/crates/core/src/refine.rs` | `RefineContext.habits` (hand-written `Debug`), `with_habits`, `habits_prompt` |
| `app/crates/core/src/ui.rs` | `TrayKind::NewHabit` |
| `app/crates/engines/src/claude.rs` | `system_blocks` takes the habits; `thinking_for`, `effort_for` crate-visible |
| `app/crates/engines/src/{chat,openrouter}.rs` | pass `&ctx.habits`; `chat::post`, `openrouter::TITLE` crate-visible |
| `app/crates/engines/src/habit.rs` | new: the rule request (`rule`, `anthropic_body`, `chat_body`) |
| `app/crates/engines/src/lib.rs` | `pub mod habit;` |
| `app/crates/cli/src/leak.rs` | the habits' examples are looked for |
| `app/src-tauri/src/runtime.rs` | counting, rule request, `CoreEvent::HabitRule`, notices in order, the card and its Undo, habits into the request, applied habits on the entry |
| `app/src-tauri/src/ipc.rs` | `Ui::habit_rule`, `SettingsView.habits`, `EntryView.habits` |
| `app/ui/src/design/drop.js` | the `habit` icon |
| `app/ui/src/{strings,trays,controller,main}.ts` | the `newHabit` card, `cue://habit-rule` |
| `app/ui/src/window/{api,page,habits,settings,history,text,icons}.ts`, `app/ui/src/design/window.css` | the Habits section, the switch moved, the History's chips |
| `app/ui/test/{windowFixture,window-habits,window-settings,window-history,controller}.test.ts` | tests |
| `README.md`, `docs/ARCHITECTURE.md` | the feature and its privacy line |

---

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

### Task 2: counting, learning, widening, declining; templates; the applied check

**Files:**
- Modify: `app/crates/core/src/habits.rs`, `app/crates/core/src/history.rs`
- Modify (an `Entry` literal each gets `habits: Vec::new()`): `app/crates/core/src/history.rs:450`,
  `app/src-tauri/src/commands.rs:849`, `app/src-tauri/src/ipc.rs:618,649,703`, `app/src-tauri/src/runtime.rs:2143,2159,2516`,
  `app/crates/cli/src/leak.rs:1124` (the compiler lists any other)

**Interfaces:**
- Consumes: Task 1.
- Produces in `history`: `#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)] pub struct HabitMark { pub id: String, pub short: String }`
  (hand-written `Debug`: id and `short_chars`); `Entry.habits: Vec<HabitMark>` (JSON `habits`, not written while empty,
  read as empty when missing).
- Produces in `habits`: `pub const LEARN_AT: u32 = 3`, `SEEN_DAYS: u32 = 30`, `MAX_SEEN: usize = 300`,
  `MAX_HABITS: usize = 50`, `MAX_RULE_CHARS: usize = 80`, `MAX_SHORT_CHARS: usize = 32`, `RULE_WAIT_MS: u64 = 5_000`;
  `pub struct Seen { pub app, pub form, pub n: u32, pub last }`;
  `pub struct Habit { pub id, pub forms: Vec<Form>, pub rule, pub short, pub before, pub after, pub app: Option<String>, pub n: u32, pub on: bool }`
  with `hashes()`, `applied(&str)`;
  `#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)] pub struct Habits { pub seen: Vec<Seen>, pub list: Vec<Habit>, pub declined: Vec<String> }`
  with `is_default`, `for_app(&str) -> impl Iterator<Item = &Habit>`, `count(&mut self, app, &Correction, today, lang) -> Counted`,
  `adopt(&mut self, Draft, Option<RuleText>, lang) -> Habit`, `forget(&mut self, id) -> bool`, `forget_all(&mut self) -> bool`,
  `switch(&mut self, id, on) -> bool`;
  `pub struct Draft { pub forms, pub app, pub before, pub after, pub n }` with `ask(&self) -> RuleAsk`;
  `pub struct Counted { pub new: Option<Draft>, pub widened: Vec<String>, pub grown: bool }`;
  `#[derive(Serialize)] pub struct RuleAsk { pub kind: Vec<&'static str>, pub before, pub after, pub app }`;
  `#[derive(Deserialize)] pub struct RuleText { pub rule: String, pub short: String }`;
  `pub fn template(&[Form], Option<&str>, lang: &str) -> RuleText`, `pub fn unscoped(rule, app, &[Form], lang) -> String`,
  `pub fn rule_messages(&RuleAsk, lang) -> (String, String)`, `pub fn read_rule(&str) -> Option<RuleText>`,
  `pub fn marks(&[Habit], text: &str) -> Vec<HabitMark>`. `lang` is the interface language's code, `"ru"` or anything
  else for English.

- [ ] **Step 1: write the failing tests** (add to `habits.rs`'s `mod tests`):

```rust
    const TG: &str = "Telegram Desktop";

    fn learned(h: &mut Habits, pasted: &str, corrected: &str, lang: &str) -> Habit {
        let c = correction(pasted, corrected);
        let mut draft = None;
        for _ in 0..LEARN_AT {
            draft = h.count(TG, &c, "2026-11-04", lang).new;
        }
        h.adopt(draft.expect("the third time"), None, lang)
    }

    #[test]
    fn three_times_in_one_app_make_one_habit_of_the_forms_together() {
        let mut h = Habits::default();
        let c = correction("Скинь файл.", "скинь файл");
        assert_eq!(h.count(TG, &c, "2026-11-02", "ru"), Counted::default());
        assert_eq!(h.seen.len(), 2);
        assert!(h.seen.iter().all(|s| s.n == 1 && s.app == TG && s.last == "2026-11-02"));
        assert_eq!(h.count(TG, &c, "2026-11-03", "ru").new, None);
        let draft = h.count(TG, &c, "2026-11-04", "ru").new.expect("the third time");
        assert_eq!(draft.forms, [Form::FirstLetter { lower: true }, STOP]);
        assert_eq!((draft.app.as_str(), draft.before.as_str(), draft.after.as_str(), draft.n), (TG, "Скинь файл.", "скинь файл", 3));
        assert!(h.seen.is_empty(), "the counts went into the habit");
        let habit = h.adopt(draft, None, "ru");
        assert_eq!(habit.rule, "В Telegram Desktop: с маленькой буквы и без точки в конце");
        assert_eq!(habit.short, "без точки");
        assert_eq!(habit.id, "h9be169a0a7be8fdd");
        assert_eq!((habit.app.as_deref(), habit.n, habit.on), (Some(TG), 3, true));
        assert_eq!(h.list, [habit]);
    }

    #[test]
    fn other_apps_count_apart_and_an_unknown_app_not_at_all() {
        let mut h = Habits::default();
        let c = correction("клиенты - в основном", "клиенты — в основном");
        h.count(TG, &c, "2026-11-04", "en");
        h.count("Word", &c, "2026-11-04", "en");
        h.count(TG, &c, "2026-11-04", "en");
        assert_eq!(h.seen.iter().map(|s| (s.app.as_str(), s.n)).collect::<Vec<_>>(), [(TG, 2), ("Word", 1)]);
        assert_eq!(h.count("", &c, "2026-11-04", "en"), Counted::default());
        assert_eq!(h.seen.len(), 2);
    }

    #[test]
    fn counts_older_than_thirty_days_go_and_at_most_three_hundred_stay() {
        let (dash, digits) = (Form::Dash.hash(), Form::Digits.hash());
        let mut h = Habits::default();
        h.seen.push(Seen { app: TG.into(), form: dash.clone(), n: 2, last: "2026-10-04".into() });
        h.seen.push(Seen { app: TG.into(), form: digits.clone(), n: 2, last: "2026-10-05".into() });
        h.count(TG, &correction("клиенты - в основном", "клиенты — в основном"), "2026-11-04", "en");
        let left: Vec<(&str, u32)> = h.seen.iter().map(|s| (s.form.as_str(), s.n)).collect();
        assert_eq!(left, [(dash.as_str(), 1), (digits.as_str(), 2)], "the dash's old count went; the digits' stays");
        for i in 0..400 {
            h.seen.push(Seen { app: format!("app{i}"), form: dash.clone(), n: 1, last: "2026-11-04".into() });
        }
        h.count("Word", &correction("рост двенадцать процентов", "рост 12 %"), "2026-11-04", "en");
        assert_eq!(h.seen.len(), MAX_SEEN);
        assert_eq!((h.seen[0].app.as_str(), h.seen[0].form.as_str()), ("Word", digits.as_str()), "the newest first");
    }

    #[test]
    fn a_habit_widens_when_its_form_reaches_three_in_another_app() {
        let mut h = Habits::default();
        let c = correction("клиенты - в основном", "клиенты — в основном");
        let mut draft = None;
        for _ in 0..LEARN_AT {
            draft = h.count(TG, &c, "2026-11-04", "ru").new;
        }
        let rule = RuleText { rule: "В Telegram Desktop — тире вместо дефиса".into(), short: "тире".into() };
        let id = h.adopt(draft.unwrap(), Some(rule), "ru").id;
        h.count("Word", &c, "2026-11-04", "ru");
        h.count("Word", &c, "2026-11-04", "ru");
        let counted = h.count("Word", &c, "2026-11-04", "ru");
        assert_eq!((counted.new, counted.widened), (None, vec![id]));
        let habit = &h.list[0];
        assert_eq!((habit.app.as_deref(), habit.n, habit.rule.as_str()), (None, 6, "Тире вместо дефиса"));
        assert!(h.seen.is_empty());
        assert!(h.count("Notepad", &c, "2026-11-04", "ru").grown, "everywhere now: a further correction counts on it");
        assert_eq!(h.list[0].n, 7);
    }

    #[test]
    fn a_rule_that_does_not_name_the_app_is_written_anew_from_the_template() {
        assert_eq!(unscoped("In Telegram Desktop: no full stop at the end", TG, &[STOP], "en"), "No full stop at the end");
        assert_eq!(unscoped("В Telegram Desktop — с маленькой буквы", TG, &[STOP], "ru"), "С маленькой буквы");
        assert_eq!(unscoped("Без точки, как вы пишете", TG, &[STOP], "ru"), "Без точки в конце");
    }

    #[test]
    fn a_forgotten_habit_is_never_learned_again_until_all_is_forgotten() {
        let mut h = Habits::default();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        assert!(h.forget(&id));
        assert!(!h.forget(&id), "already gone");
        assert_eq!(h.declined, [Form::FirstLetter { lower: true }.hash(), STOP.hash()]);
        let c = correction("Скинь файл.", "скинь файл");
        for _ in 0..5 {
            assert_eq!(h.count(TG, &c, "2026-11-04", "en"), Counted::default());
        }
        assert!(h.seen.is_empty(), "a declined form is not even counted");
        assert!(h.forget_all());
        assert_eq!(h, Habits::default());
        assert!(!h.forget_all(), "nothing left");
    }

    #[test]
    fn a_habit_switched_off_stays_but_goes_into_no_request() {
        let mut h = Habits::default();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        assert_eq!(h.for_app(TG).count(), 1);
        assert_eq!(h.for_app("Word").count(), 0, "another app's");
        assert_eq!(h.for_app("").count(), 0, "an unknown app gets only the habits of everywhere");
        assert!(h.switch(&id, false));
        assert!(!h.switch(&id, false), "no change");
        assert_eq!(h.for_app(TG).count(), 0);
        assert!(!h.switch("nope", true));
    }

    #[test]
    fn at_most_fifty_habits_the_newest_first() {
        let mut h = Habits::default();
        for i in 0..MAX_HABITS + 2 {
            let forms = vec![Form::Replacement { from: format!("a{i}"), to: format!("b{i}") }];
            h.adopt(Draft { forms, app: TG.into(), before: format!("a{i}"), after: format!("b{i}"), n: 3 }, None, "en");
        }
        assert_eq!(h.list.len(), MAX_HABITS);
        assert_eq!(h.list[0].after, format!("b{}", MAX_HABITS + 1));
    }

    #[test]
    fn templates_say_the_rule_and_its_short_name() {
        let t = |forms: &[Form], app: Option<&str>, lang: &str| {
            let r = template(forms, app, lang);
            (r.rule, r.short)
        };
        assert_eq!(t(&[STOP], Some("Telegram"), "en"), ("In Telegram: no full stop at the end".into(), "no full stop".into()));
        assert_eq!(t(&[STOP], Some("Telegram"), "ru"), ("В Telegram: без точки в конце".into(), "без точки".into()));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert_eq!(t(&[hello], None, "ru"), ("Писать «добрый день» вместо «здравствуйте»".into(), "добрый день".into()));
        assert_eq!(t(&[Form::Digits], None, "en").0, "Numbers in digits");
        assert_eq!(t(&[Form::Dash], Some("Docs"), "ru").0, "В Docs: тире вместо дефиса");
        assert_eq!(
            t(&[Form::FirstLetter { lower: true }, STOP], Some("Telegram"), "en").0,
            "In Telegram: lower case at the start and no full stop at the end",
        );
        let long = Form::Replacement { from: "я".repeat(40), to: "ю".repeat(40) };
        let r = template(&[long], Some("Telegram"), "ru");
        assert!(r.rule.chars().count() <= MAX_RULE_CHARS && r.short.chars().count() <= MAX_SHORT_CHARS);
    }

    #[test]
    fn the_rule_request_holds_the_kind_the_example_and_the_app_only() {
        let forms = vec![Form::FirstLetter { lower: true }, STOP];
        let d = Draft { forms, app: "Telegram".into(), before: "Скинь файл.".into(), after: "скинь файл".into(), n: 3 };
        let (system, user) = rule_messages(&d.ask(), "ru");
        let v: serde_json::Value = serde_json::from_str(&user).unwrap();
        let expected = serde_json::json!({ "kind": ["firstLetter", "endMark"], "before": "Скинь файл.", "after": "скинь файл", "app": "Telegram" });
        assert_eq!(v, expected);
        assert!(system.contains("Write both in Russian.") && !system.contains("Скинь") && !system.contains("скинь"));
        assert!(rule_messages(&d.ask(), "en").0.contains("Write both in English."));
    }

    #[test]
    fn only_a_usable_answer_is_taken() {
        let ok = read_rule(r#"{"rule":"В Telegram — с маленькой буквы и без точки.","short":"без точки"}"#).unwrap();
        assert_eq!((ok.rule.as_str(), ok.short.as_str()), ("В Telegram — с маленькой буквы и без точки", "без точки"));
        assert!(read_rule("```json\n{\"rule\":\"No full stop\",\"short\":\"no stop\"}\n```").is_some(), "in a code fence");
        let long = format!(r#"{{"rule":"{}","short":"x"}}"#, "a".repeat(MAX_RULE_CHARS + 1));
        for bad in [
            r#"{"rule":"","short":"x"}"#,
            long.as_str(),
            r#"{"rule":"two\nlines","short":"x"}"#,
            r#"{"rule":"fine","short":"one two three four"}"#,
            "not json",
            r#"{"text":"fine"}"#,
        ] {
            assert!(read_rule(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_applied_check_per_kind() {
        assert!(STOP.shows_in("скинь файл") && !STOP.shows_in("Скинь файл."));
        assert!(Form::EndMark { mark: '.', removed: false }.shows_in("Готово."));
        let lower = Form::FirstLetter { lower: true };
        assert!(lower.shows_in("«скинь» файл") && !lower.shows_in("Скинь файл"));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert!(hello.shows_in("Добрый день, Анна!"));
        assert!(!hello.shows_in("Здравствуйте, Анна!") && !hello.shows_in("Добрый день! Здравствуйте, Анна"));
        assert!(Form::Digits.shows_in("рост 12 %") && !Form::Digits.shows_in("рост двенадцать процентов"));
        assert!(Form::Dash.shows_in("клиенты — в основном") && !Form::Dash.shows_in("клиенты - в основном"));
    }

    #[test]
    fn only_habits_whose_every_form_shows_are_marked() {
        let habit = |id: &str, forms: Vec<Form>| Habit {
            id: id.into(),
            forms,
            rule: "r".into(),
            short: format!("s{id}"),
            before: String::new(),
            after: String::new(),
            app: None,
            n: 3,
            on: true,
        };
        let sent = [habit("1", vec![Form::FirstLetter { lower: true }, STOP]), habit("2", vec![Form::Digits])];
        assert_eq!(marks(&sent, "скинь файл"), [HabitMark { id: "1".into(), short: "s1".into() }]);
        assert!(marks(&sent, "Скинь файл").is_empty());
    }

    #[test]
    fn the_lists_read_leniently_and_print_no_text() {
        let json = serde_json::json!({
            "seen": [{ "app": "Telegram Desktop", "form": "94128fbe83ca55c4", "n": 2, "last": "2026-11-04" }, { "app": 1 }],
            "list": [
                { "id": "h1", "forms": [{ "kind": "endMark", "mark": ".", "removed": true }], "rule": "В Telegram: без точки в конце",
                  "short": "без точки", "before": "Скинь файл.", "after": "скинь файл", "app": "Telegram Desktop", "n": 3 },
                { "id": "h2", "forms": [{ "kind": "later" }] }
            ],
            "declined": ["85657e6730abe435", 7]
        });
        let h: Habits = serde_json::from_value(json).unwrap();
        assert_eq!((h.seen.len(), h.list.len(), h.declined.len()), (1, 1, 1));
        assert!(h.list[0].on, "on unless written off");
        let shown = format!("{h:?}");
        for word in ["Скинь", "скинь", "Telegram", "точки"] {
            assert!(!shown.contains(word), "{word} in {shown}");
        }
        let h: Habits = serde_json::from_value(serde_json::json!({ "seen": 5, "list": "x" })).unwrap();
        assert_eq!(h, Habits::default());
        assert_eq!(serde_json::to_value(Habits::default()).unwrap(), serde_json::json!({}));
    }
```

  In `history.rs`'s tests, next to `entry_debug_has_no_text` (line 1018):

```rust
    /// The habits that shaped an entry: written only when there are some, read as none from an earlier line, and
    /// shown in a log only by their count.
    #[test]
    fn an_entry_keeps_the_habits_that_shaped_it() {
        let mut e = entry("2026-11-04T10:00:00.000", Mode::Dictation, "скинь файл");
        let line = serde_json::to_value(&e).unwrap();
        assert!(line.get("habits").is_none());
        e.habits = vec![HabitMark { id: "h1".into(), short: "без точки".into() }];
        let line = serde_json::to_string(&e).unwrap();
        assert!(line.contains(r#""habits":[{"id":"h1","short":"без точки"}]"#), "{line}");
        assert_eq!(serde_json::from_str::<Entry>(&line).unwrap(), e);
        assert!(!format!("{e:?}").contains("точки"));
    }
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core`; expected: `Habits`, `HabitMark` and the rest
  are not defined.

- [ ] **Step 3: `history.rs`.** Above `pub struct Entry` (line 38):

```rust
/// A habit whose effect showed in an entry's text (docs/specs/2026-11-04-habits.md): its id, and its short name as it
/// was then, so the entry's chip stays when the habit is forgotten.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitMark {
    pub id: String,
    pub short: String,
}

/// A short name may hold the words of a replacement: only its length.
impl std::fmt::Debug for HabitMark {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HabitMark").field("id", &self.id).field("short_chars", &self.short.chars().count()).finish()
    }
}
```

  In `Entry`, after `raw`, and the sentence "`habits` are the habits whose effect showed in a dictation's text."
  at the end of its doc comment:

```rust
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub habits: Vec<HabitMark>,
```

  In its `Debug`, after `raw_chars`: `.field("habits", &self.habits.len())`. Add `habits: Vec::new()` to every `Entry`
  literal the compiler names (listed under Files).

- [ ] **Step 4: the state, the templates and the check** (`habits.rs`, after `impl Correction`; the imports gain
  `Deserializer` and `crate::history::{self, HabitMark}`):

```rust
/// A form becomes a habit when it was corrected this many times in one app.
pub const LEARN_AT: u32 = 3;
/// A count not seen for this many days is dropped.
pub const SEEN_DAYS: u32 = 30;
/// The most counts kept; the least recently seen go first.
pub const MAX_SEEN: usize = 300;
/// The most habits kept; the oldest go first.
pub const MAX_HABITS: usize = 50;
/// The longest rule, and short name, a habit takes.
pub const MAX_RULE_CHARS: usize = 80;
pub const MAX_SHORT_CHARS: usize = 32;
/// How long the rule request may take before the template writes the rule.
pub const RULE_WAIT_MS: u64 = 5_000;

/// How many times a form was corrected in an app, and the day it last was: no text (`form` is its hash).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub app: String,
    pub form: String,
    pub n: u32,
    pub last: String,
}

/// The app's name is the user's: only its length.
impl std::fmt::Debug for Seen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seen")
            .field("app_chars", &self.app.chars().count())
            .field("form", &self.form)
            .field("n", &self.n)
            .field("last", &self.last)
            .finish()
    }
}

fn yes() -> bool {
    true
}

/// A habit: its forms (each kind with its detail), the rule the cleanup follows and its short name, the example, the
/// app it holds in (`None`: everywhere), how many times it was corrected, and whether it is on.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Habit {
    pub id: String,
    pub forms: Vec<Form>,
    pub rule: String,
    pub short: String,
    pub before: String,
    pub after: String,
    #[serde(default)]
    pub app: Option<String>,
    pub n: u32,
    #[serde(default = "yes")]
    pub on: bool,
}

impl std::fmt::Debug for Habit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Habit")
            .field("id", &self.id)
            .field("forms", &self.forms)
            .field("rule_chars", &self.rule.chars().count())
            .field("short_chars", &self.short.chars().count())
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .field("everywhere", &self.app.is_none())
            .field("n", &self.n)
            .field("on", &self.on)
            .finish()
    }
}

impl Habit {
    pub fn hashes(&self) -> Vec<String> {
        self.forms.iter().map(Form::hash).collect()
    }

    /// Its effect shows in `text`: every one of its forms does.
    pub fn applied(&self, text: &str) -> bool {
        !self.forms.is_empty() && self.forms.iter().all(|f| f.shows_in(text))
    }
}

/// A list of items as settings.json holds them: anything but a list reads as none, and an item that does not read (a
/// later version's, a typo) is skipped.
fn items_or_nothing<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else { return Ok(Vec::new()) };
    Ok(items.into_iter().filter_map(|item| serde_json::from_value(item).ok()).collect())
}

/// `habits` in settings.json: the counts (`seen`, hashes only), the habits (`list`, the newest first) and the forms
/// never to learn again (`declined`, hashes only). Each list is written only when it is not empty.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Habits {
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<Seen>,
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<Habit>,
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub declined: Vec<String>,
}

/// A habit being made: its forms, the app and the example, and how many times they were corrected. Its rule comes
/// from the cleanup's connection or the template ([`Habits::adopt`]).
#[derive(Clone, PartialEq)]
pub struct Draft {
    pub forms: Vec<Form>,
    pub app: String,
    pub before: String,
    pub after: String,
    pub n: u32,
}

impl std::fmt::Debug for Draft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Draft")
            .field("forms", &self.forms)
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .field("n", &self.n)
            .finish()
    }
}

/// What the rule request carries, and nothing else: the kinds, the example and the app's name.
#[derive(Clone, PartialEq, Serialize)]
pub struct RuleAsk {
    pub kind: Vec<&'static str>,
    pub before: String,
    pub after: String,
    pub app: String,
}

impl std::fmt::Debug for RuleAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleAsk")
            .field("kind", &self.kind)
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .finish()
    }
}

impl Draft {
    pub fn ask(&self) -> RuleAsk {
        RuleAsk { kind: self.forms.iter().map(Form::kind).collect(), before: self.before.clone(), after: self.after.clone(), app: self.app.clone() }
    }
}

/// A habit's rule, a sentence of at most `MAX_RULE_CHARS`, and its short name, one to three words.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct RuleText {
    pub rule: String,
    pub short: String,
}

impl std::fmt::Debug for RuleText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleText")
            .field("rule_chars", &self.rule.chars().count())
            .field("short_chars", &self.short.chars().count())
            .finish()
    }
}

/// What counting one correction changed.
#[derive(Debug, Default, PartialEq)]
pub struct Counted {
    /// Forms that reached `LEARN_AT` in this app together: the habit to make.
    pub new: Option<Draft>,
    /// Habits of another app whose form reached `LEARN_AT` here: they hold everywhere now.
    pub widened: Vec<String>,
    /// A habit's form was corrected again: its count grew.
    pub grown: bool,
}

impl Habits {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The habits that go into a dictation's cleanup request in `app`: on, and for that app or everywhere (an unknown
    /// app, `""`, gets the ones of everywhere).
    pub fn for_app<'a>(&'a self, app: &'a str) -> impl Iterator<Item = &'a Habit> + 'a {
        self.list.iter().filter(move |h| h.on && h.app.as_deref().is_none_or(|a| !app.is_empty() && a == app))
    }

    /// One correction of a dictation in `app` (the history's name for it, `today` its local date): each form counts
    /// once. A declined form is not counted. A form already a habit here or everywhere adds to that habit's count; one
    /// that reaches `LEARN_AT` and is a habit of another app widens it (its rule loses its app, its counts add up); the
    /// others that reach it together become one draft. The counts that went into a habit go; counts older than
    /// `SEEN_DAYS` go, and past `MAX_SEEN` the least recently seen.
    pub fn count(&mut self, app: &str, c: &Correction, today: &str, lang: &str) -> Counted {
        let mut out = Counted::default();
        if app.is_empty() {
            return out;
        }
        self.prune(today);
        let mut reached: Vec<&Found> = Vec::new();
        for f in &c.found {
            let hash = f.form.hash();
            if self.declined.contains(&hash) {
                continue;
            }
            if let Some(h) = self.list.iter_mut().find(|h| h.hashes().contains(&hash) && h.app.as_deref().is_none_or(|a| a == app)) {
                h.n = h.n.saturating_add(1);
                out.grown = true;
                continue;
            }
            let n = self.bump(app, &hash, today);
            if n < LEARN_AT {
                continue;
            }
            if let Some(h) = self.list.iter_mut().find(|h| h.hashes().contains(&hash)) {
                let was = h.app.take().unwrap_or_default();
                h.rule = unscoped(&h.rule, &was, &h.forms, lang);
                h.n = h.n.saturating_add(n);
                self.seen.retain(|s| s.form != hash);
                out.widened.push(h.id.clone());
                continue;
            }
            reached.push(f);
        }
        if !reached.is_empty() {
            let gone: Vec<String> = reached.iter().map(|f| f.form.hash()).collect();
            self.seen.retain(|s| s.app != app || !gone.contains(&s.form));
            let mut forms: Vec<Form> = reached.iter().map(|f| f.form.clone()).collect();
            forms.sort_by_key(Form::rank);
            let (before, after) = c.example(&reached);
            out.new = Some(Draft { forms, app: app.to_owned(), before, after, n: LEARN_AT });
        }
        out
    }

    /// The form's count in `app` grows by one and goes first (the most recently seen); the new count.
    fn bump(&mut self, app: &str, form: &str, today: &str) -> u32 {
        let n = match self.seen.iter().position(|s| s.app == app && s.form == form) {
            Some(i) => self.seen.remove(i).n.saturating_add(1),
            None => 1,
        };
        self.seen.insert(0, Seen { app: app.to_owned(), form: form.to_owned(), n, last: today.to_owned() });
        self.seen.truncate(MAX_SEEN);
        n
    }

    /// Counts last seen before the `SEEN_DAYS` before `today` go (none when `today` is not a date).
    fn prune(&mut self, today: &str) {
        if let Some(first_kept) = history::date_minus(today, SEEN_DAYS) {
            self.seen.retain(|s| s.last.as_str() >= first_kept.as_str());
        }
    }

    /// The draft's habit goes first, with the rule the cleanup wrote or else the template's; past `MAX_HABITS` the
    /// oldest go. Its id is the hash of its forms (a form is in one habit at most: a forgotten one is declined).
    pub fn adopt(&mut self, draft: Draft, rule: Option<RuleText>, lang: &str) -> Habit {
        let RuleText { rule, short } = rule.unwrap_or_else(|| template(&draft.forms, Some(&draft.app), lang));
        let hashes: Vec<String> = draft.forms.iter().map(Form::hash).collect();
        let id = format!("h{:016x}", fnv1a(hashes.join(",").as_bytes()));
        let habit = Habit { id, forms: draft.forms, rule, short, before: draft.before, after: draft.after, app: Some(draft.app), n: draft.n, on: true };
        self.list.retain(|h| h.id != habit.id);
        self.list.insert(0, habit.clone());
        self.list.truncate(MAX_HABITS);
        habit
    }

    /// The habit goes, and its forms are declined: never learned again. False when there is no such habit.
    pub fn forget(&mut self, id: &str) -> bool {
        let Some(i) = self.list.iter().position(|h| h.id == id) else { return false };
        for hash in self.list.remove(i).hashes() {
            if !self.declined.contains(&hash) {
                self.declined.push(hash);
            }
        }
        true
    }

    /// Every habit goes, with the counts and the declined forms. False when there was nothing.
    pub fn forget_all(&mut self) -> bool {
        let changed = !self.is_default();
        *self = Self::default();
        changed
    }

    /// A habit on or off; false when nothing changed.
    pub fn switch(&mut self, id: &str, on: bool) -> bool {
        self.list.iter_mut().find(|h| h.id == id).is_some_and(|h| std::mem::replace(&mut h.on, on) != on)
    }
}

/// `s` with its first letter a capital.
fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|first| first.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// One form's part of a template rule, and its short name.
fn part(form: &Form, ru: bool) -> (String, String) {
    let own = |rule: &str, short: &str| (rule.to_owned(), short.to_owned());
    match (form, ru) {
        (Form::EndMark { mark: '.', removed: true }, false) => own("no full stop at the end", "no full stop"),
        (Form::EndMark { mark: '.', removed: true }, true) => own("без точки в конце", "без точки"),
        (Form::EndMark { mark, removed: true }, false) => (format!("no “{mark}” at the end"), format!("no “{mark}”")),
        (Form::EndMark { mark, removed: true }, true) => (format!("без «{mark}» в конце"), format!("без «{mark}»")),
        (Form::EndMark { mark: '.', removed: false }, false) => own("a full stop at the end", "full stop"),
        (Form::EndMark { mark: '.', removed: false }, true) => own("с точкой в конце", "с точкой"),
        (Form::EndMark { mark, removed: false }, false) => (format!("“{mark}” at the end"), format!("with “{mark}”")),
        (Form::EndMark { mark, removed: false }, true) => (format!("«{mark}» в конце"), format!("с «{mark}»")),
        (Form::FirstLetter { lower: true }, false) => own("lower case at the start", "lower case"),
        (Form::FirstLetter { lower: true }, true) => own("с маленькой буквы", "с маленькой"),
        (Form::FirstLetter { lower: false }, false) => own("a capital at the start", "capital"),
        (Form::FirstLetter { lower: false }, true) => own("с большой буквы", "с большой"),
        (Form::Replacement { from, to }, false) => (format!("write “{to}” instead of “{from}”"), cut(to, MAX_SHORT_CHARS, false)),
        (Form::Replacement { from, to }, true) => (format!("писать «{to}» вместо «{from}»"), cut(to, MAX_SHORT_CHARS, false)),
        (Form::Digits, false) => own("numbers in digits", "digits"),
        (Form::Digits, true) => own("числа цифрами", "цифрами"),
        (Form::Dash, false) => own("a dash between words, not a hyphen", "dash"),
        (Form::Dash, true) => own("тире вместо дефиса", "тире"),
    }
}

/// The rule when the cleanup's connection gave none: "In <app>: …" / «В <app>: …» with each form's part (the last
/// joined by "and" / «и»), or without the app for everywhere; the short name is the last form's. At most
/// `MAX_RULE_CHARS`.
pub fn template(forms: &[Form], app: Option<&str>, lang: &str) -> RuleText {
    let ru = lang == "ru";
    let parts: Vec<(String, String)> = forms.iter().map(|f| part(f, ru)).collect();
    let and = if ru { " и " } else { " and " };
    let what = match parts.as_slice() {
        [] => String::new(),
        [one] => one.0.clone(),
        [init @ .., last] => format!("{}{and}{}", init.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", "), last.0),
    };
    let rule = match app {
        Some(app) => format!("{} {app}: {what}", if ru { "В" } else { "In" }),
        None => capitalized(&what),
    };
    let short = parts.last().map(|p| p.1.clone()).unwrap_or_default();
    RuleText { rule: cut(&rule, MAX_RULE_CHARS, false), short }
}

/// A rule written for one app, for every app: its "In <app>" / «В <app>» start cut off (with a dash, a colon or a
/// comma after it), the rest from a capital; a rule that does not start so is written anew from the template.
pub fn unscoped(rule: &str, app: &str, forms: &[Form], lang: &str) -> String {
    for lead in ["In ", "in ", "В ", "в "] {
        if let Some(rest) = rule.strip_prefix(lead).and_then(|r| r.strip_prefix(app)) {
            let rest = rest.trim_start_matches([' ', ':', '—', '–', '-', ',']);
            if !rest.is_empty() {
                return capitalized(rest);
            }
        }
    }
    template(forms, None, lang).rule
}

/// The rule request's instructions; `{LANGUAGE}` becomes the interface language.
const RULE_PROMPT: &str = "\
A dictation app pastes text that the user then corrects by hand, and the user has made the same correction three \
times in one app. From the kind of correction (endMark: the last sentence mark removed or added; firstLetter: the first \
letter's case changed; replacement: words written another way; digits: numbers written as digits; dash: a dash between \
words instead of a hyphen), one example (the text before and after the correction) and the app's name, write the rule \
the app should follow there from now on. \"rule\": one sentence of at most 80 characters that starts with the app, like \
\"In Telegram — lower case and no full stop\". \"short\": one to three words that name it, like \"no full stop\". No full \
stop at the end of either. Write both in {LANGUAGE}. Answer with a JSON object with exactly the fields \"rule\" and \
\"short\".";

/// The rule request's two messages: the instructions, and the user message — the kinds, the example and the app as a
/// JSON object, nothing else.
pub fn rule_messages(ask: &RuleAsk, lang: &str) -> (String, String) {
    let language = if lang == "ru" { "Russian" } else { "English" };
    (RULE_PROMPT.replace("{LANGUAGE}", language), serde_json::to_string(ask).unwrap_or_default())
}

/// One line: no control character, no line or paragraph separator.
fn one_line(s: &str) -> bool {
    !s.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

/// The cleanup's answer to the rule request: a JSON object with `rule` and `short`, in a code fence or not. Usable when
/// each is one line, the rule not empty and at most `MAX_RULE_CHARS`, the short name one to three words and at most
/// `MAX_SHORT_CHARS`; a full stop at their end goes.
pub fn read_rule(content: &str) -> Option<RuleText> {
    let t = content.trim();
    let t = match t.strip_prefix("```json").or_else(|| t.strip_prefix("```")) {
        Some(inner) => inner.trim_end().strip_suffix("```").unwrap_or(inner),
        None => t,
    };
    let r: RuleText = serde_json::from_str(t.trim()).ok()?;
    let rule = r.rule.trim().trim_end_matches('.').trim_end().to_owned();
    let short = r.short.trim().trim_end_matches('.').trim_end().to_owned();
    let words = short.split_whitespace().count();
    let ok = !rule.is_empty()
        && rule.chars().count() <= MAX_RULE_CHARS
        && one_line(&rule)
        && (1..=3).contains(&words)
        && short.chars().count() <= MAX_SHORT_CHARS
        && one_line(&short);
    ok.then_some(RuleText { rule, short })
}

/// Of the habits sent with a dictation's cleanup, the ones whose effect shows in the cleaned text: its history entry
/// lists them.
pub fn marks(sent: &[Habit], text: &str) -> Vec<HabitMark> {
    sent.iter().filter(|h| h.applied(text)).map(|h| HabitMark { id: h.id.clone(), short: h.short.clone() }).collect()
}
```

  Task 4's `habits_prompt` checks a rule with `one_line` too, and makes it `pub(crate)`.

- [ ] **Step 5: run** `cargo test -p cue-core` and `cargo clippy -p cue-core --all-targets -- -D warnings`: green. Then
  `cargo build --workspace --all-targets`: the `Entry` literals outside the crate need `habits: Vec::new()`; follow the
  compiler.

---

### Task 3: settings — the habits section and its three changes

**Files:**
- Modify: `app/crates/core/src/settings.rs`, `app/crates/cli/src/leak.rs`

**Interfaces:**
- Consumes: `habits::Habits` (Task 2).
- Produces: `Settings.habits: Habits` (JSON `habits`, not written while empty, a section that is not an object reads
  as empty); `SettingChange::HabitOn { id: String, on: bool }` (JSON `{"kind":"habitOn","value":{"id":…,"on":…}}`),
  `SettingChange::HabitForget(String)` (`{"kind":"habitForget","value":"h…"}`), `SettingChange::HabitsForget`
  (`{"kind":"habitsForget"}`).

- [ ] **Step 1: write the failing tests** next to `learning_is_on_by_default_and_only_off_is_written`:

```rust
    /// The habits keep their own section, written only when there is something in it; the window changes one at a time.
    #[test]
    fn habits_are_kept_in_their_section_and_changed_one_at_a_time() {
        let mut s = Settings::default();
        assert!(serde_json::to_value(&s).unwrap().get("habits").is_none(), "nothing written while there are none");
        let d = crate::habits::Draft { forms: vec![crate::habits::Form::Dash], app: "Word".into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        let id = s.habits.adopt(d, None, "en").id;
        assert_eq!(serde_json::to_value(&s).unwrap()["habits"]["list"][0]["after"], "a — b");
        assert!(s.apply(SettingChange::HabitOn { id: id.clone(), on: false }));
        assert!(!s.apply(SettingChange::HabitOn { id: id.clone(), on: false }), "no change");
        assert!(!s.habits.list[0].on);
        assert!(s.apply(SettingChange::HabitForget(id.clone())));
        assert_eq!(s.habits.declined, [crate::habits::Form::Dash.hash()]);
        assert!(!s.apply(SettingChange::HabitForget(id)), "already gone");
        assert!(s.apply(SettingChange::HabitsForget));
        assert!(!s.apply(SettingChange::HabitsForget), "nothing left");
        let read = |json: serde_json::Value| serde_json::from_value::<SettingChange>(json).unwrap();
        assert_eq!(read(serde_json::json!({ "kind": "habitOn", "value": { "id": "h1", "on": true } })), SettingChange::HabitOn { id: "h1".into(), on: true });
        assert_eq!(read(serde_json::json!({ "kind": "habitForget", "value": "h1" })), SettingChange::HabitForget("h1".into()));
        assert_eq!(read(serde_json::json!({ "kind": "habitsForget" })), SettingChange::HabitsForget);
        let s: Settings = serde_json::from_str(r#"{"habits":"nonsense"}"#).unwrap();
        assert!(s.habits.is_default());
    }

    /// A habit's example and rule are the user's words: neither the settings nor a change show them in a log.
    #[test]
    fn debug_shows_no_habit_text() {
        let mut s = Settings::default();
        let d = crate::habits::Draft {
            forms: vec![crate::habits::Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() }],
            app: "Почта".into(),
            before: "Здравствуйте, Анна!".into(),
            after: "Добрый день, Анна!".into(),
            n: 3,
        };
        let id = s.habits.adopt(d, None, "ru").id;
        let shown = format!("{s:?} {:?} {:?}", SettingChange::HabitOn { id: id.clone(), on: false }, SettingChange::HabitForget(id));
        for word in ["Анна", "добрый", "здравствуйте", "Почта"] {
            assert!(!shown.contains(word), "{word} in a log");
        }
        assert_eq!(format!("{:?}", SettingChange::HabitsForget), "HabitsForget");
    }
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core settings`.

- [ ] **Step 3: implement.** `use crate::habits::Habits;` at the top (line 5 block). In `Settings` (line 859), after
  `learn`:

```rust
    /// Habits (docs/specs/2026-11-04-habits.md): counts, habits and declined forms. Not written while empty.
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "Habits::is_default")]
    pub habits: Habits,
```

  and `habits: Habits::default(),` in `impl Default for Settings` (line 940). In `SettingChange` (line 675), after
  `Learn`:

```rust
    /// A habit on or off (docs/specs/2026-11-04-habits.md).
    HabitOn { id: String, on: bool },
    /// A habit forgotten from the main window or its card's "Undo": its forms are never learned again.
    HabitForget(String),
    /// "Forget all": every habit, the counts and the declined forms.
    HabitsForget,
```

  `Debug` arms (line 715; ids are hashes, never text):

```rust
            Self::HabitOn { id, on } => f.debug_struct("HabitOn").field("id", id).field("on", on).finish(),
            Self::HabitForget(id) => f.debug_tuple("HabitForget").field(id).finish(),
            Self::HabitsForget => f.write_str("HabitsForget"),
```

  `apply` arms (line 754), and the sentence "`HabitOn` switches a habit, `HabitForget` forgets one and declines its
  forms, `HabitsForget` forgets them all; each is refused when it changes nothing." at the end of its doc comment:

```rust
            SettingChange::HabitOn { id, on } => self.habits.switch(&id, on),
            SettingChange::HabitForget(id) => self.habits.forget(&id),
            SettingChange::HabitsForget => self.habits.forget_all(),
```

  Add ", the habits" to the module's first sentence after "the user's dictionary".

- [ ] **Step 4: the leak scan looks for the examples.** In `app/crates/cli/src/leak.rs`: a kind `HabitExample(usize)`
  after `DictionaryTerm(usize)` (line 67), its `Display` arm `Kind::HabitExample(i) => write!(f, "habit example {}", i + 1),`,
  in `summary` (line 136) "habit examples {}" after "dictionary terms {}" with
  `self.count(|k| matches!(k, Kind::HabitExample(_))),`, and in `gather` after the learned words (line 183):

```rust
        // A habit's example is the user's text; its rule and short name are written from it.
        for (i, habit) in settings.habits.list.iter().enumerate() {
            for text in [&habit.before, &habit.after, &habit.rule, &habit.short] {
                private.add(Kind::HabitExample(i), text, MIN_NAME_CHARS);
            }
        }
```

  Update the module comment's list ("the dictionary terms, the habits' examples and rules and the microphone names (4
  or longer)") and any test that pins `summary()`'s text.

- [ ] **Step 5: run** `cargo test --workspace` and clippy: green.

---

### Task 4: the habits block in the cleanup request (Clean and Rewrite only)

**Files:**
- Modify: `app/crates/core/src/refine.rs` (`RefineContext`, line 126), `app/crates/engines/src/claude.rs`
  (`system_blocks`, line 103; `warm_body`, line 137), `chat.rs` (line 46), `openrouter.rs` (line 214)
- Modify (a `RefineContext` literal each gets `..Default::default()`): `refine.rs:605,607,714,959`, `chat.rs:196`,
  `claude.rs:430`, `openrouter.rs:773`

**Interfaces:**
- Consumes: `habits::MAX_RULE_CHARS` (Task 2).
- Produces: `RefineContext.habits: Vec<String>` (the rules), `#[derive(Clone, PartialEq, Default)]` with a hand-written
  `Debug`; `pub fn with_habits(self, rules: Vec<String>) -> Self`; `pub fn habits_prompt(rules: &[String]) -> Option<String>`;
  `pub(crate) fn system_blocks(dict: &DictionarySettings, habits: &[String], job: Job, custom: Option<&str>) -> Value`.

- [ ] **Step 1: write the failing tests.** In `refine.rs`'s tests, after `no_usable_terms_means_no_block`:

```rust
    #[test]
    fn habits_prompt_lists_the_rules_one_per_line() {
        let rules = vec!["В Telegram — с маленькой буквы и без точки".to_owned(), " ".to_owned(), "two\nlines".to_owned()];
        assert_eq!(habits_prompt(&rules).unwrap(), format!("{HABITS_INTRO}\n\nВ Telegram — с маленькой буквы и без точки"));
        assert_eq!(habits_prompt(&[]), None);
        assert_eq!(habits_prompt(&["x".repeat(crate::habits::MAX_RULE_CHARS + 1)]), None);
    }

    #[test]
    fn the_context_carries_the_habits_and_prints_no_text() {
        let ctx = RefineContext::new("Telegram Desktop", Some("секрет"), 500).with_habits(vec!["без точки".into()]);
        assert_eq!(ctx.habits, ["без точки"]);
        let shown = format!("{ctx:?}");
        assert!(!shown.contains("секрет") && !shown.contains("точки") && !shown.contains("Telegram"), "{shown}");
    }
```

  In `claude.rs`'s tests (`dict` helper at line 425; add `use cue_core::refine::habits_prompt;`):

```rust
    /// Habits go after the dictionary, behind the cache mark, for a cleanup only; with none the body is the same.
    #[test]
    fn habits_go_after_the_dictionary_for_a_cleanup_only() {
        let rules = vec!["В Telegram — с маленькой буквы и без точки".to_owned()];
        let with = RefineContext { habits: rules.clone(), ..ctx() };
        let block = habits_prompt(&rules).unwrap();
        let v = request_body_for("claude-haiku-5-5", "привет", &with, &dict(&["Soniox"]), Job::Rewrite);
        let texts: Vec<&str> = v["system"].as_array().unwrap().iter().map(|b| b["text"].as_str().unwrap()).collect();
        assert_eq!((texts.len(), texts[0], texts[2], texts[3]), (4, SYSTEM_PROMPT, block.as_str(), BOLD_PROMPT));
        assert!(v["system"][1]["cache_control"].is_object() && v["system"][2].get("cache_control").is_none(), "the cached prefix is the one without habits");
        assert_eq!(request_body("claude-haiku-5-5", "привет", &with, &dict(&[]))["system"][1]["text"], block);
        let to_en = request_body_for("claude-haiku-5-5", "привет", &with, &dict(&[]), Job::Translate("en"));
        assert!(!to_en["system"].to_string().contains("без точки"), "a translation takes none");
        let none = RefineContext { habits: Vec::new(), ..ctx() };
        assert_eq!(request_body("claude-haiku-5-5", "привет", &none, &dict(&[])), request_body("claude-haiku-5-5", "привет", &ctx(), &dict(&[])));
    }
```

  In `chat.rs`'s tests, after `the_style_and_the_translation_blocks_are_claude_s`:

```rust
    #[test]
    fn habits_ride_in_the_system_message() {
        let c = find("openai").unwrap();
        let with = RefineContext { habits: vec!["No full stop".into()], ..ctx() };
        let v = request_body(c, "gpt-6-luna", RAW, &with, &Default::default(), Job::Rewrite, None);
        let block = cue_core::refine::habits_prompt(&with.habits).unwrap();
        assert_eq!(v["messages"][0]["content"], format!("{SYSTEM_PROMPT}\n\n{block}\n\n{BOLD_PROMPT}"));
    }
```

- [ ] **Step 2: run and see them fail** — `cargo test -p cue-core refine` and `cargo test -p cue-engines`.

- [ ] **Step 3: implement in `refine.rs`.** Next to `TERMS_INTRO`:

```rust
/// The start of the habits block ([`habits_prompt`]); the rules follow, one per line.
const HABITS_INTRO: &str = "The user's habits; follow them unless the speech clearly asks otherwise:";

/// The habits block (docs/specs/2026-11-04-habits.md): a system block after the dictionary's, one rule per line. A rule
/// that is empty, not one line or longer than `habits::MAX_RULE_CHARS` is skipped; `None` when none is left, and the
/// request is then exactly the one without habits.
pub fn habits_prompt(rules: &[String]) -> Option<String> {
    let usable: Vec<&str> = rules
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty() && r.chars().count() <= crate::habits::MAX_RULE_CHARS && crate::habits::one_line(r))
        .collect();
    (!usable.is_empty()).then(|| format!("{HABITS_INTRO}\n\n{}", usable.join("\n")))
}
```

  (make `habits::one_line` `pub(crate)`). `RefineContext` becomes:

```rust
/// The request context: the app, the text before the caret, and the rules of the user's habits for this dictation
/// (docs/specs/2026-11-04-habits.md; empty for a translation and "Ask").
#[derive(Clone, PartialEq, Default)]
pub struct RefineContext {
    pub app: String,
    pub before_cursor: Option<String>,
    pub habits: Vec<String>,
}

/// The app, the text and the rules are the user's: only their lengths.
impl std::fmt::Debug for RefineContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefineContext")
            .field("app_chars", &self.app.chars().count())
            .field("before_cursor_chars", &self.before_cursor.as_ref().map(|b| b.chars().count()))
            .field("habits", &self.habits.len())
            .finish()
    }
}
```

  In `RefineContext::new`: `Self { app: app.into(), before_cursor, habits: Vec::new() }`, and add:

```rust
    /// The context with the rules of the habits sent with this dictation.
    pub fn with_habits(mut self, rules: Vec<String>) -> Self {
        self.habits = rules;
        self
    }
```

- [ ] **Step 4: the engines.** In `claude.rs` import `habits_prompt`, and `system_blocks` (line 103) becomes:

```rust
/// The system blocks of a refine request: the prompt, the dictionary block ([`terms_prompt`]) with the cache mark on the
/// last of them; then, for a cleanup, the habits block ([`habits_prompt`]); then the job's own block: CUE's cleanup
/// ([`BOLD_PROMPT`]) or the user's instructions (`custom`), or the translation's. Everything after the mark changes
/// with the app or the job, so the cached prefix is always the same.
pub(crate) fn system_blocks(dict: &DictionarySettings, habits: &[String], job: Job, custom: Option<&str>) -> Value {
    let mut system = match terms_prompt(&dict.terms, &dict.learned) {
        None => vec![json!({ "type": "text", "text": SYSTEM_PROMPT, "cache_control": { "type": "ephemeral" } })],
        Some(block) => vec![
            json!({ "type": "text", "text": SYSTEM_PROMPT }),
            json!({ "type": "text", "text": block, "cache_control": { "type": "ephemeral" } }),
        ],
    };
    if matches!(job, Job::Clean | Job::Rewrite)
        && let Some(block) = habits_prompt(habits)
    {
        system.push(json!({ "type": "text", "text": block }));
    }
    match job {
        Job::Clean => {}
        Job::Rewrite => {
            let block = custom.map_or_else(|| BOLD_PROMPT.to_owned(), |text| format!("{CUSTOM_PREAMBLE}{text}"));
            system.push(json!({ "type": "text", "text": block }))
        }
        Job::Translate(to) => system.push(json!({ "type": "text", "text": translate::instruction(to) })),
    }
    Value::Array(system)
}
```

  Its three callers pass the context's rules: `claude.rs:75` `system_blocks(dict, &ctx.habits, job, custom)`,
  `chat.rs:46` and `openrouter.rs:214` `claude::system_blocks(dict, &ctx.habits, job, custom)`. `warm_body` (line
  137): `let ctx = RefineContext::default();`. Every test literal `RefineContext { app: …, before_cursor: … }` gets
  `..Default::default()`.

- [ ] **Step 5: run** `cargo test --workspace` and clippy: green. The request with no habits stays byte for byte the same
  (the existing request-body tests prove it).

---

### Task 5: the rule request through the cleanup's connection (`cue-engines::habit`)

**Files:**
- Create: `app/crates/engines/src/habit.rs`
- Modify: `app/crates/engines/src/lib.rs` (`pub mod habit;` after `pub mod error;`), `claude.rs` (`thinking_for`,
  line 20, and `effort_for`, line 31, become `pub(crate)`), `chat.rs` (`post`, line 100, becomes `pub(crate)`),
  `openrouter.rs` (`TITLE`, line 23, becomes `pub(crate)`)

**Interfaces:**
- Consumes: `habits::{RuleAsk, RuleText, rule_messages, read_rule}` (Task 2).
- Produces: `pub async fn rule(http: &reqwest::Client, key: Option<&str>, s: &RefineSettings, ask: &RuleAsk, lang: &str, timeout: Duration) -> Option<RuleText>`
  (`None`: no key, refused, no answer within `timeout`, or none usable; one log line with the reason, never text);
  `pub fn anthropic_body(model: &str, system: &str, user: &str) -> Value`; `pub fn chat_body(s: &RefineSettings, system: &str, user: &str) -> Value`.

- [ ] **Step 1: write the failing tests** (end of `habit.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{capture_logs, fake_api, Reply};
    use cue_core::habits::{Draft, Form};

    const KEY: &str = "sk-HABIT-TEST-0123456789";
    const WAIT: Duration = Duration::from_secs(5);

    fn ask() -> RuleAsk {
        let forms = vec![Form::FirstLetter { lower: true }, Form::EndMark { mark: '.', removed: true }];
        Draft { forms, app: "Telegram".into(), before: "Скинь файл.".into(), after: "скинь файл".into(), n: 3 }.ask()
    }
    fn over(id: &str, base: &str) -> RefineSettings {
        let mut s = RefineSettings { connection: id.into(), ..RefineSettings::default() };
        s.addresses.insert(id.into(), base.into());
        s
    }
    fn http() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }
    /// The only words of the user's in a request: the example and the app, in the user message.
    fn only_the_ask(user: &str) {
        let v: Value = serde_json::from_str(user).unwrap();
        assert_eq!(v, json!({ "kind": ["firstLetter", "endMark"], "before": "Скинь файл.", "after": "скинь файл", "app": "Telegram" }));
    }

    #[tokio::test]
    async fn anthropic_gets_the_kind_the_example_and_the_app_and_its_rule_comes_back() {
        let text = json!({ "rule": "В Telegram — с маленькой буквы и без точки", "short": "без точки" }).to_string();
        let answer = json!({ "content": [{ "type": "text", "text": text }], "stop_reason": "end_turn", "usage": {} }).to_string();
        let api = fake_api(move |_, _, _| Reply::Send(200, answer.clone(), "")).await;
        let s = over("anthropic", &format!("{}/v1", api.base));
        let r = rule(&http(), Some(KEY), &s, &ask(), "ru", WAIT).await.unwrap();
        assert_eq!((r.rule.as_str(), r.short.as_str()), ("В Telegram — с маленькой буквы и без точки", "без точки"));
        assert_eq!(api.calls(), ["POST /v1/messages"]);
        let sent: Value = serde_json::from_slice(&api.body(0)).unwrap();
        let (system, user) = habits::rule_messages(&ask(), "ru");
        assert_eq!(sent, anthropic_body(s.model().id, &system, &user));
        assert_eq!(sent["messages"].as_array().unwrap().len(), 1);
        only_the_ask(sent["messages"][0]["content"].as_str().unwrap());
    }

    #[tokio::test]
    async fn a_chat_service_gets_the_same_and_streams_its_rule() {
        let chunk = |content: &str, finish: Option<&str>| {
            json!({ "id": "c1", "model": "gpt-6-luna", "choices": [{ "index": 0, "delta": { "content": content }, "finish_reason": finish }] }).to_string()
        };
        let text = json!({ "rule": "In Telegram — no full stop", "short": "no full stop" }).to_string();
        let body: String = [chunk(&text, None), chunk("", Some("stop")), "[DONE]".to_owned()].iter().map(|e| format!("data: {e}\n\n")).collect();
        let api = fake_api(move |_, _, _| Reply::Send(200, body.clone(), "")).await;
        let s = over("openai", &format!("{}/v1", api.base));
        let r = rule(&http(), Some(KEY), &s, &ask(), "en", WAIT).await.unwrap();
        assert_eq!(r.short, "no full stop");
        assert_eq!(api.calls(), ["POST /v1/chat/completions"]);
        let sent: Value = serde_json::from_slice(&api.body(0)).unwrap();
        assert_eq!(sent, chat_body(&s, &habits::rule_messages(&ask(), "en").0, &habits::rule_messages(&ask(), "en").1));
        assert_eq!(sent["messages"].as_array().unwrap().len(), 2);
        only_the_ask(sent["messages"][1]["content"].as_str().unwrap());
        assert_eq!(sent["response_format"]["json_schema"]["name"], "habit_rule");
    }

    #[tokio::test]
    async fn no_key_no_answer_in_time_or_an_unusable_one_give_none_and_log_no_text() {
        let (logs, _guard) = capture_logs();
        assert!(rule(&http(), None, &RefineSettings::default(), &ask(), "en", WAIT).await.is_none(), "no key: not even asked");
        let api = fake_api(|_, _, _| Reply::Hang).await;
        let s = over("anthropic", &format!("{}/v1", api.base));
        let t0 = std::time::Instant::now();
        assert!(rule(&http(), Some(KEY), &s, &ask(), "en", Duration::from_millis(200)).await.is_none());
        assert!(t0.elapsed() < Duration::from_secs(2), "the wait ends the request");
        let bad = json!({ "content": [{ "type": "text", "text": "{\"rule\":\"\",\"short\":\"\"}" }], "stop_reason": "end_turn" }).to_string();
        let api = fake_api(move |_, _, _| Reply::Send(200, bad.clone(), "")).await;
        assert!(rule(&http(), Some(KEY), &over("anthropic", &format!("{}/v1", api.base)), &ask(), "en", WAIT).await.is_none());
        for line in logs.lock().unwrap().iter().filter(|l| l.contains("cue_engines::habit")) {
            assert!(!line.contains("Скинь") && !line.contains("скинь") && !line.contains(KEY), "{line}");
        }
    }
}
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-engines habit`.

- [ ] **Step 3: implement** (`habit.rs`, above `mod tests`):

```rust
//! The rule of a habit just learned (docs/specs/2026-11-04-habits.md), asked of the cleanup's connection: one short
//! request with the kind of the correction, its example and the app's name, nothing else (`habits::rule_messages`).
//! Never fails: `None` on any problem, with one log line of the reason, and the caller writes the rule from the
//! template. The key goes only in its header; the answer never reaches the log.
use std::time::Duration;

use cue_core::connections::{Answer, Kind};
use cue_core::habits::{self, RuleAsk, RuleText};
use cue_core::settings::RefineSettings;
use serde_json::{json, Value};

use crate::{chat, claude, openrouter};

/// The answer is a sentence and a few words.
const RULE_TOKENS: u32 = 200;

/// The answer's shape: the rule and its short name.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "rule": { "type": "string" }, "short": { "type": "string" } },
        "required": ["rule", "short"],
        "additionalProperties": false
    })
}

/// The Messages API body: the instructions, the user message, the answer in the schema, effort and thinking as the
/// cleanup sets them for the model.
pub fn anthropic_body(model: &str, system: &str, user: &str) -> Value {
    let thinking = claude::thinking_for(model);
    let budget = RULE_TOKENS + if thinking.is_none() { claude::THINKING_HEADROOM } else { 0 };
    let mut v = json!({
        "model": model,
        "max_tokens": budget,
        "system": system,
        "messages": [{ "role": "user", "content": user }],
        "output_config": { "effort": claude::effort_for(model), "format": { "type": "json_schema", "schema": schema() } }
    });
    if let Some(t) = thinking {
        v["thinking"] = t;
    }
    v
}

/// The Chat Completions body for OpenRouter and the OpenAI-compatible services, with what each needs as the cleanup's
/// requests have it: the reasoning setting, the budget's headroom, the answer's format.
pub fn chat_body(s: &RefineSettings, system: &str, user: &str) -> Value {
    let (c, model) = (s.connection(), s.model().id);
    let named = |strict: bool| json!({ "type": "json_schema", "json_schema": { "name": "habit_rule", "strict": strict, "schema": schema() } });
    let mut v = json!({
        "model": model,
        "messages": [{ "role": "system", "content": system }, { "role": "user", "content": user }],
        "stream": true
    });
    if matches!(c.kind, Kind::OpenRouter) {
        let p = openrouter::model_profile(model);
        v["max_tokens"] = json!(RULE_TOKENS + if p.reasoning.mandatory() { openrouter::REASONING_HEADROOM } else { 0 });
        if let Some(r) = p.reasoning.param() {
            v["reasoning"] = r;
        }
        if p.structured {
            v["response_format"] = named(true);
        }
    } else {
        let budget = RULE_TOKENS + if c.reasons { openrouter::REASONING_HEADROOM } else { 0 };
        v[if c.completion_tokens { "max_completion_tokens" } else { "max_tokens" }] = json!(budget);
        if let Some(effort) = c.reasoning {
            v["reasoning_effort"] = json!(effort);
        }
        v["response_format"] = match c.answer {
            Answer::Schema { strict } => named(strict),
            Answer::Object => json!({ "type": "json_object" }),
        };
    }
    v
}

/// No rule from the connection: why, in the log, and the template writes it.
fn skip(why: &str) -> Option<RuleText> {
    tracing::info!("habit rule: {why}, written from the template");
    None
}

/// The rule from the cleanup's connection in `s` (its address, its model), within `timeout`; a local server needs no
/// key, a cloud one without a key is not asked.
pub async fn rule(http: &reqwest::Client, key: Option<&str>, s: &RefineSettings, ask: &RuleAsk, lang: &str, timeout: Duration) -> Option<RuleText> {
    let c = s.connection();
    let key = match key {
        Some(k) => k,
        None if c.local => "",
        None => return skip(&format!("no {} key", c.name)),
    };
    let (system, user) = habits::rule_messages(ask, lang);
    let content = match c.kind {
        Kind::Anthropic => {
            let url = format!("{}/messages", s.address());
            let body = anthropic_body(s.model().id, &system, &user);
            let resp = match claude::messages_post(http, &url, key).timeout(timeout).json(&body).send().await {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => return skip(&format!("HTTP {}", r.status().as_u16())),
                Err(e) => return skip(if e.is_timeout() { "timeout" } else { "network" }),
            };
            match resp.json::<claude::MessageResp>().await {
                Ok(m) => m.content.into_iter().find(|b| b.kind == "text").and_then(|b| b.text).unwrap_or_default(),
                Err(_) => return skip("response not parsed"),
            }
        }
        Kind::OpenRouter | Kind::Chat => {
            let req = if matches!(c.kind, Kind::OpenRouter) {
                http.post(format!("{}/chat/completions", s.address())).bearer_auth(key).header("X-Title", openrouter::TITLE)
            } else {
                chat::post(http, c, s.address(), key)
            };
            match openrouter::stream(req, key, &chat_body(s, &system, &user), timeout).await {
                Ok(done) => done.content,
                Err(f) => return skip(&openrouter::reason(&f)),
            }
        }
    };
    habits::read_rule(&content).or_else(|| skip("the answer is not a usable rule"))
}
```

  `claude::MessageResp` and its `Block` are already `pub`; `openrouter::stream` and `reason` are `pub(crate)`.

- [ ] **Step 4: run** `cargo test -p cue-engines` and `cargo clippy --workspace --all-targets -- -D warnings`: green.

---

### Task 6: the runtime — counting, the rule, the card at rest, Undo, widening, the applied habits

**Files:**
- Modify: `app/crates/core/src/ui.rs` (`TrayKind`, line 44), `app/crates/core/src/corrections.rs` (`Offers`, line 257),
  `app/src-tauri/src/ipc.rs` (`Ui`, next to `model_name`, line 370), `app/src-tauri/src/runtime.rs`

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces: `TrayKind::NewHabit` (JSON `"newHabit"`); `Offers::waiting(&self) -> bool`, `Offers::open(&self) -> bool`;
  `Ui::habit_rule(&self, rule: &str)` emitting `cue://habit-rule` (a string) to the Drop just before the card;
  `CoreEvent::Corrected { pasted, span, stop, app: Option<String> }` (`app`: the dictation's app for habits, `None` for a
  translation); `CoreEvent::HabitRule { draft: u64, rule: Option<habits::RuleText> }`; the card's "Undo" is
  `UiAction::TrayAction` on `TrayKind::NewHabit` → `SettingChange::HabitForget`; `cue://settings` after a habit is
  learned, widened, grown or forgotten from the card.

- [ ] **Step 1: the failing tests.** In `ui.rs`'s `every_variant_has_its_json_name`, add `(TrayKind::NewHabit, "newHabit")`
  to `trays`. In `corrections.rs`'s tests:

```rust
    #[test]
    fn waiting_and_open_say_where_the_suggestion_is() {
        let mut o = Offers::default();
        assert!(!o.waiting() && !o.open());
        o.offer(marisol());
        assert!(o.waiting() && !o.open());
        o.at_rest();
        assert!(!o.waiting() && o.open());
        o.added();
        assert!(o.open(), "showing undo");
        o.done();
        o.offer(marisol());
        o.at_rest();
        o.dismissed();
        assert!(!o.waiting() && !o.open(), "a missed one is a dot");
    }
```

  In `runtime.rs`'s tests:

```rust
    /// The suggestion and a habit's card at rest: the one found first; a card never while the capsule is open.
    #[test]
    fn the_notice_found_first_opens_first() {
        assert_eq!(first_notice(Some(1), Some(2), false), Some(Notice::Offer));
        assert_eq!(first_notice(Some(3), Some(2), false), Some(Notice::Habit));
        assert_eq!(first_notice(None, Some(2), true), None, "the capsule is open");
        assert_eq!(first_notice(None, Some(2), false), Some(Notice::Habit));
        assert_eq!(first_notice(Some(1), None, false), Some(Notice::Offer));
        assert_eq!(first_notice(None, None, false), None);
    }
```

  Run `cargo test -p cue-core` and `cargo test -p cue` and see them fail.

- [ ] **Step 2: the kind, the offer's state, the push.** In `ui.rs` after `ModelSwitched`:

```rust
    /// A habit was just learned (docs/specs/2026-11-04-habits.md): its rule comes to the Drop just before the card, and
    /// the card's button undoes it.
    NewHabit,
```

  In `corrections.rs`, in `impl Offers` after `view`:

```rust
    /// Found while the Drop was busy: it opens at the next rest.
    pub fn waiting(&self) -> bool {
        matches!(self.cur, Some((_, Stage::Waiting)))
    }

    /// The capsule is open: with its ring, or showing "undo".
    pub fn open(&self) -> bool {
        matches!(self.cur, Some((_, Stage::Open | Stage::Added)))
    }
```

  In `ipc.rs` after `model_name`:

```rust
    /// The rule of the habit just learned, just before its card (docs/specs/2026-11-04-habits.md): to the Drop, which
    /// draws it.
    pub fn habit_rule(&self, rule: &str) {
        if let Err(e) = self.app.emit_to(DROP_LABEL, "cue://habit-rule", rule) {
            tracing::debug!("Drop window: event cue://habit-rule not sent: {e}");
        }
    }
```

- [ ] **Step 3: the runtime's state.** Imports: `use std::collections::VecDeque;`, `use cue_core::habits;`,
  `use cue_core::flow::RefineOutcome` (into the `cue_core::flow` line 66), `use cue_engines::habit;`. `CoreEvent::Corrected`
  (line 150) gains `app: Option<String>` ("the dictation's app, when its correction counts for habits: `None` for a
  translation"), and after it:

```rust
    /// The rule of the habit being made under `draft`, from the cleanup's connection: `None` (no connection, no answer
    /// in 5 s, none usable) — the template writes it. Never logged.
    HabitRule { draft: u64, rule: Option<habits::RuleText> },
```

  `watch_next` (line 285) becomes `Option<(DictationId, String, Option<String>)>` ("…with its text, and its app when
  its correction counts for habits"). In `struct Runtime`, after `field_stop`:

```rust
    /// Habits (docs/specs/2026-11-04-habits.md): the ones sent with a dictation's cleanup, and those whose effect showed
    /// in its text (its history entry lists them).
    habits_sent: Option<(DictationId, Vec<habits::Habit>)>,
    habits_applied: Option<(DictationId, Vec<history::HabitMark>)>,
    /// Habits waiting for their rule, by the number they were found with.
    drafts: Vec<(u64, habits::Draft)>,
    /// Habit cards waiting for the Drop's rest, in the order found, and the habit whose card is open.
    habit_cards: VecDeque<(u64, String)>,
    habit_card: Option<String>,
    /// Numbers the notices as they are found (`next_notice`), the suggestion's (`offer_found`) and each habit's: at rest
    /// the one found first opens first.
    notices: u64,
    offer_found: u64,
```

  initialized `habits_sent: None, habits_applied: None, drafts: Vec::new(), habit_cards: VecDeque::new(), habit_card: None, notices: 0, offer_found: 0,`
  in `Runtime::new` (after `field_stop: None,`, line 383).

- [ ] **Step 4: a dictation's paste is watched for habits too.** In `insert` (line 1322), the `watch_next` line
  becomes:

```rust
        // A dictation's correction counts for habits in its app as the history names it; a translation's only offers
        // words.
        let habit_app = (self.attempt.mode == Mode::Dictate).then(|| stored_app(&self.attempt.app)).filter(|a| !a.is_empty());
        self.watch_next = (self.settings.learn.enabled() && self.attempt.mode != Mode::Ask).then(|| (id, text.clone(), habit_app));
```

  In `InsertDone` (line 588): `if let Some((wanted, pasted, app)) = self.watch_next.take()` … `self.watch_field(pasted, app);`.
  `watch_field` (line 767) takes `app: Option<String>` and sends `CoreEvent::Corrected { pasted, span, stop: own, app }`.
  The `Corrected` arm (line 596): `CoreEvent::Corrected { pasted, span, stop, app } => self.on_corrected(&pasted, span.as_deref(), &stop, app.as_deref()),`.
  And the new arm next to it: `CoreEvent::HabitRule { draft, rule } => self.on_habit_rule(draft, rule),`.

- [ ] **Step 5: counting, the rule, the habit.** `on_corrected` (line 785) becomes:

```rust
    /// A watch is over: a word the user wrote instead of the one heard becomes the suggestion (the first, when there
    /// are several), and a dictation's correction counts for habits (`count_habits`). The log gets numbers, never the
    /// text.
    fn on_corrected(&mut self, pasted: &str, span: Option<&str>, stop: &Arc<AtomicBool>, app: Option<&str>) {
        // A watch replaced by a later one leaves the later one's flag in place.
        self.field_stop.take_if(|running| Arc::ptr_eq(running, stop));
        if !self.settings.learn.enabled() {
            return;
        }
        let Some(span) = span else {
            tracing::info!("correction: none");
            return;
        };
        let found = corrections::candidates(pasted, span, &self.settings.dictionary.terms);
        tracing::info!(chars = span.chars().count(), candidates = found.len(), "correction found");
        if let Some(first) = found.into_iter().next() {
            let before = self.offers.view();
            self.offers.offer(first);
            if self.offers.waiting() {
                self.offer_found = self.next_notice();
            }
            // A missed one it replaced (the dot) is gone from the page until the new one opens at rest.
            let after = self.offers.view();
            if after != before {
                self.ui.suggest(after.as_ref());
            }
        }
        if let Some(app) = app {
            self.count_habits(app, pasted, span);
        }
        self.show_notice();
    }

    /// The forms of a dictation's correction in `app` are counted (`Habits::count`): a habit of another app whose form
    /// reached three here holds everywhere now (its row changes, no card), and forms that reached three here together
    /// become a habit once its rule comes (`ask_rule`). The log gets kinds and numbers.
    fn count_habits(&mut self, app: &str, pasted: &str, span: &str) {
        let c = habits::correction(pasted, span);
        if c.found.is_empty() {
            return;
        }
        let counted = self.settings.habits.count(app, &c, &local_today(), self.lang.code());
        self.save_settings();
        let kinds: Vec<&str> = c.found.iter().map(|f| f.form.kind()).collect();
        tracing::info!(?kinds, widened = counted.widened.len(), new = counted.new.is_some(), "habit forms counted");
        if counted.grown || !counted.widened.is_empty() {
            self.send_settings();
        }
        if let Some(draft) = counted.new {
            self.ask_rule(draft);
        }
    }

    /// The rule of a habit just found, asked of the cleanup's connection: within `RULE_WAIT_MS`, or at once without a
    /// connection, `on_habit_rule` gets it or nothing. The habit is numbered now, as found.
    fn ask_rule(&mut self, draft: habits::Draft) {
        let n = self.next_notice();
        let ask = draft.ask();
        self.drafts.push((n, draft));
        let (http, s, tx, lang, key) = (self.http.clone(), self.settings.refine.clone(), self.tx.clone(), self.lang.code(), self.cleanup_key());
        tokio::spawn(async move {
            let wait = Duration::from_millis(habits::RULE_WAIT_MS);
            let rule = tokio::time::timeout(wait, habit::rule(&http, key.as_deref(), &s, &ask, lang, wait)).await.ok().flatten();
            let _ = tx.send(CoreEvent::HabitRule { draft: n, rule });
        });
    }

    /// The rule came (or the template writes it): the habit is saved, the main window draws it, and its card waits for
    /// the Drop's rest. Dropped when "Learn from me" went off meanwhile.
    fn on_habit_rule(&mut self, n: u64, rule: Option<habits::RuleText>) {
        let Some(at) = self.drafts.iter().position(|(d, _)| *d == n) else { return };
        let (_, draft) = self.drafts.remove(at);
        if !self.settings.learn.enabled() {
            return;
        }
        let from_cleanup = rule.is_some();
        let habit = self.settings.habits.adopt(draft, rule, self.lang.code());
        self.save_settings();
        let kinds: Vec<&str> = habit.forms.iter().map(habits::Form::kind).collect();
        tracing::info!(?kinds, n = habit.n, from_cleanup, "habit learned");
        self.send_settings();
        self.habit_cards.push_back((n, habit.id));
        self.show_notice();
    }

    fn next_notice(&mut self) -> u64 {
        self.notices += 1;
        self.notices
    }

    /// The settings to the main window after a change it did not ask for.
    fn send_settings(&self) {
        self.ui.settings(&SettingsView::of(&self.settings, self.lang, self.can_save_settings));
    }

    /// The cleanup connection's key from the store (a local server has none; `None` too when it cannot be read).
    fn cleanup_key(&self) -> Option<String> {
        KeyName::of_connection(self.settings.refine.connection().id).and_then(|name| {
            keys::lookup(name).unwrap_or_else(|e| {
                tracing::warn!("{} key: {e}", name.as_str());
                None
            })
        })
    }
```

  In `refine` (line 1260) replace the key lookup with `let key = self.cleanup_key();`, and in `learn_word` (line 757)
  replace its `self.ui.settings(…)` with `self.send_settings()`.

- [ ] **Step 6: one notice at a time, the one found first.** `show_offer` (line 812) becomes `show_notice`; rename
  its five callers (`feed`, the `TrayClose` arm, `on_corrected`, `on_tray_action`, the `IdleHidden` arm):

```rust
    /// A notice waiting opens once the Drop is at rest and can be clicked (`at_rest`): the dictionary suggestion or a
    /// habit's card, one at a time, the one found first (`first_notice`). Called again after anything that may end a
    /// busy Drop: every input (the end of a pause among them), a tray card going, the idle Drop showing again once no
    /// full-screen app is in front, a habit learned.
    fn show_notice(&mut self) {
        let s = self.flow.ui_state();
        if !at_rest(s.phase, self.tray.is_some(), self.flow.answer_open(), s.paused, self.idle_hidden) {
            return;
        }
        let offer = self.offers.waiting().then_some(self.offer_found);
        let card = self.habit_cards.front().map(|(n, _)| *n);
        match first_notice(offer, card, self.offers.open()) {
            Some(Notice::Offer) => {
                if self.offers.at_rest() {
                    self.ui.suggest(self.offers.view().as_ref());
                }
            }
            Some(Notice::Habit) => self.show_habit_card(),
            None => {}
        }
    }

    /// The next habit's card: its rule to the Drop, then the card. One forgotten from the main window meanwhile has none.
    fn show_habit_card(&mut self) {
        while let Some((_, id)) = self.habit_cards.pop_front() {
            let Some(rule) = self.settings.habits.list.iter().find(|h| h.id == id).map(|h| h.rule.clone()) else { continue };
            self.habit_card = Some(id);
            self.ui.habit_rule(&rule);
            self.tray = Some(TrayKind::NewHabit);
            self.ui.tray(self.tray);
            return;
        }
    }

    /// The rule of the habit whose card is open, before the card goes to a Drop loaded again.
    fn send_habit_rule(&self, kind: Option<TrayKind>) {
        if kind == Some(TrayKind::NewHabit)
            && let Some(h) = self.habit_card.as_ref().and_then(|id| self.settings.habits.list.iter().find(|h| &h.id == id))
        {
            self.ui.habit_rule(&h.rule);
        }
    }
```

  Free functions next to `at_rest` (line 2342):

```rust
/// Which notice opens at rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Notice {
    Offer,
    Habit,
}

/// The one found first of a waiting suggestion and a habit's card, each by the number it was found with; a card never
/// opens while the suggestion's capsule is open (`open`: with its ring or "undo"). A missed suggestion's dot does not
/// hold a card back.
fn first_notice(offer: Option<u64>, card: Option<u64>, open: bool) -> Option<Notice> {
    match (offer, card) {
        (Some(o), Some(c)) if c < o => (!open).then_some(Notice::Habit),
        (Some(_), _) => Some(Notice::Offer),
        (None, Some(_)) => (!open).then_some(Notice::Habit),
        (None, None) => None,
    }
}
```

  Update `at_rest`'s doc comment: "The Drop is at rest for a notice (the suggestion, a habit's card): …".

- [ ] **Step 7: the card's Undo, and cards that go.** In `on_tray_action` (line 1832), a new arm before the catch-all:

```rust
            // "Undo" on a habit's card: the habit goes, and its forms are never learned again.
            Some(TrayKind::NewHabit) => {
                if let Some(id) = self.habit_card.take() {
                    self.learn_word(SettingChange::HabitForget(id));
                    tracing::info!("habit undone");
                }
            }
```

  The `TrayClose` arm (line 511) sets `self.habit_card = None;` before `show_notice()`. In `exec`'s `Effect::Tray(kind)`
  arm (line 971), before `self.tray = kind;`: `self.habit_card = None; // the flow's card, or none, takes a habit's place`.
  In `UiReady` (line 554), before `self.ui.tray(self.tray);`: `self.send_habit_rule(self.tray);`. In `change_setting`'s
  "Learn from me" off block (line 744), add `self.drafts.clear();` and `self.habit_cards.clear();` (an open card stays
  until it closes; its Undo still forgets).

- [ ] **Step 8: the habits go with a dictation's cleanup, and the applied ones on its entry.** In `refine` (line 1260),
  after `let job = job_for(…)` and before the task is spawned:

```rust
        // The habits of this app (or of everywhere) go with a dictation's cleanup, never with a translation or "Ask".
        let sent: Vec<habits::Habit> = if mode == Mode::Dictate && self.settings.learn.enabled() && matches!(job, Job::Clean | Job::Rewrite) {
            self.settings.habits.for_app(&stored_app(&self.attempt.app)).cloned().collect()
        } else {
            Vec::new()
        };
        let ctx = ctx.with_habits(sent.iter().map(|h| h.rule.clone()).collect());
        self.habits_sent = Some((id, sent));
```

  In `on_refined` (line 897), after the `finished` check:

```rust
        // Of the habits sent, those whose effect shows in the cleaned text go on its history entry; a raw fallback shows
        // none.
        if let Some((_, sent)) = self.habits_sent.take_if(|(i, _)| *i == id) {
            let marks = if result.outcome == RefineOutcome::Refined { habits::marks(&sent, &result.text) } else { Vec::new() };
            self.habits_applied = Some((id, marks));
        }
```

  In `on_finished`'s `LastFiles::Commit` branch (line 1887):

```rust
                let mut entry = dictation_entry(&self.attempt, self.final_text.as_ref(), raw.as_ref(), stamp);
                if let Some(e) = entry.as_mut()
                    && let Some((_, marks)) = self.habits_applied.take_if(|(i, _)| *i == id)
                {
                    e.habits = marks;
                }
```

  Add a bullet to the module comment after the corrections one: "a dictation's correction also counts for habits
  (docs/specs/2026-11-04-habits.md): a form corrected three times in one app becomes a habit, its rule asked of the
  cleanup's connection (5 s, else a template), its card shown at rest after the suggestion found before it; the
  habits of the dictation's app go with its cleanup, and those whose effect shows are kept on its history entry;".

- [ ] **Step 9: run** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`; `cargo build -p cue`
  builds.

---

### Task 7: the card on the Drop (`newHabit`)

**Files:**
- Modify: `app/ui/src/design/drop.js` (`IC`, line 14), `app/ui/src/strings.ts`, `app/ui/src/trays.ts`,
  `app/ui/src/controller.ts`, `app/ui/src/main.ts` (line 65)
- Test: `app/ui/test/controller.test.ts`

**Interfaces:**
- Consumes: `cue://tray` `"newHabit"`, `cue://habit-rule` (a string), `trayAction` / `trayClose` as for the other cards.
- Produces: `TrayKind` gains `'newHabit'`; `TRAYS.newHabit = { icon: 'habit', tone: 'ok', act: 'undo', ttl: 7000 }`;
  `DropController.habitRule(rule: string)`; `trayHtml(kind, t, names?, model?, rule?)` (`{RULE}` in a body, escaped);
  `CUE.IC.habit`.

- [ ] **Step 1: the failing tests.** In `controller.test.ts` add `'newHabit'` to `KINDS` (line 24), and after the
  "Tray card closes on a timer" block:

```ts
describe('New habit card', () => {
  const box = () => document.querySelector('.tray') as HTMLElement;
  const isOpen = () => box().classList.contains('open');
  const act = () => box().querySelector('[data-act]') as HTMLButtonElement;
  const RULE = 'In Telegram — lower case and no full stop';

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('says the rule under "New habit", with the habit icon in green', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    expect(box().querySelector('.nhd b')!.textContent).toBe('New habit');
    expect(box().querySelector('.nbd')!.textContent).toBe(`${RULE}. CUE writes it this way from now on.`);
    const nic = box().querySelector('.nic')!;
    expect(nic.classList.contains('tone-ok')).toBe(true);
    const icon = document.createElement('span');
    icon.innerHTML = CUE().IC.habit;
    expect(nic.innerHTML).toBe(icon.innerHTML);
    expect(act().querySelector('.lbl')!.getAttribute('data-t')).toBe('Undo');
  });

  it('closes by itself after 7 s; hovering pauses it', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    box().dispatchEvent(new MouseEvent('mouseenter'));
    vi.advanceTimersByTime(30_000);
    expect(isOpen()).toBe(true);
    box().dispatchEvent(new MouseEvent('mouseleave'));
    vi.advanceTimersByTime(7000 - 1);
    expect(isOpen()).toBe(true);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('"Undo" turns into "Removed", tells the core, and the card closes a moment later', () => {
    ctl.habitRule(RULE);
    ctl.tray('newHabit');
    act().click();
    expect(sent).toEqual(['trayAction']);
    const letters = [...box().querySelectorAll('.lbl span')].map((s) => s.textContent).join('');
    expect(letters).toBe('Removed');
    expect(act().disabled).toBe(true);
    vi.advanceTimersByTime(1100);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayAction']);
  });

  it('the rule goes in as text', () => {
    ctl.habitRule('<b>x</b>');
    ctl.tray('newHabit');
    expect(box().querySelector('.nbd b')).toBeNull();
    expect(box().querySelector('.nbd')!.textContent).toContain('<b>x</b>');
  });
});
```

  And in the "Russian interface" block:

```ts
  it('the habit card in Russian: «Отменить» becomes «Убрано»', () => {
    ctl.habitRule('В Telegram — с маленькой буквы и без точки');
    ctl.tray('newHabit');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Новая привычка');
    expect(document.querySelector('.tray .nbd')!.textContent).toBe('В Telegram — с маленькой буквы и без точки. Дальше CUE пишет так сам.');
    (document.querySelector('.tray [data-act]') as HTMLButtonElement).click();
    expect([...document.querySelectorAll('.tray .lbl span')].map((s) => s.textContent).join('')).toBe('Убрано');
  });
```

  Run `npx vitest run test/controller.test.ts` (from `app`) and see them fail.

- [ ] **Step 2: the icon.** In `drop.js`'s `IC`, after `book`, drawn on the 20 grid with the 1.8 stroke (design stage 5's
  habit icon, a pen with a plus):

```js
 habit:SV('<path d="M4 16.2c2.2-.2 3.6-1 4.8-2.2l6.6-6.6a1.6 1.6 0 0 0-2.3-2.3L6.5 11.7C5.3 12.9 4.5 14.3 4.3 16"/><path d="M15.6 12.6v3M14.1 14.1h3"/>'),
```

- [ ] **Step 3: `strings.ts`.** `TrayKind` (line 4) gains `| 'newHabit'`; `TrayLook.icon` gains `| 'habit'`,
  `TrayLook.act` gains `| 'undo'`; `TrayText.body`'s comment adds "{RULE} the habit's rule, escaped"; `TRAYS` gains
  `newHabit: { icon: 'habit', tone: 'ok', act: 'undo', ttl: 7000 },`; `en.tray` gains

```ts
    newHabit: {
      title: 'New habit',
      body: '{RULE}. CUE writes it this way from now on.',
      act: 'Undo', done: 'Removed',
    },
```

  and `ru.tray`

```ts
    newHabit: {
      title: 'Новая привычка',
      body: '{RULE}. Дальше CUE пишет так сам.',
      act: 'Отменить', done: 'Убрано',
    },
```

- [ ] **Step 4: `trays.ts`.** `trayHtml` takes `rule?: string | null` after `model`, and its body chain gains
  `.replace('{RULE}', () => escapeHtml(rule ?? ''))`; its doc comment says "{RULE} the habit's rule".

- [ ] **Step 5: `controller.ts`.** After `private model` (line 76): `private rule: string | null = null;`; after
  `modelName` (line 278):

```ts
  /** The rule of the habit just learned, for the card that follows. */
  habitRule(rule: string) {
    this.rule = String(rule ?? '');
  }
```

  and line 285: `trayHtml(kind, this.t, this.mic, this.model, this.rule)`. The existing `done` path gives "Undo" →
  "Removed" by shared letters and closes the card after `DONE_HOLD_MS`; nothing else changes.

- [ ] **Step 6: `main.ts`.** After the `cue://model-name` listener:
  `listen<string>('cue://habit-rule', (e) => ctl.habitRule(e.payload)),`.

- [ ] **Step 7: run** `npx vitest run` and `npx tsc --noEmit -p tsconfig.json` from `app`: green.

---

### Task 8: the main window's Habits section (section A)

**Files:**
- Modify: `app/src-tauri/src/ipc.rs` (`SettingsView`, line 38; `of`, line 114; `Debug`, line 140)
- Create: `app/ui/src/window/habits.ts`
- Modify: `app/ui/src/window/{api,page,settings,text,icons}.ts`, `app/ui/src/design/window.css`
- Test: `app/ui/test/windowFixture.ts`, `app/ui/test/window-habits.test.ts` (new), `app/ui/test/window-settings.test.ts`

**Interfaces:**
- Consumes: `SettingChange::{Learn, HabitOn, HabitForget, HabitsForget}`, `cue://settings`.
- Produces: `SettingsView.habits: Vec<HabitView>` (`{ id, rule, before, after, app, n, on }`, `app` null for
  everywhere); TS `HabitView`, `SettingChange` gains `{ kind: 'habitOn'; value: { id: string; on: boolean } }`,
  `{ kind: 'habitForget'; value: string }`, `{ kind: 'habitsForget' }`; `HabitsView` with `mount(sec)`, `render(animate?)`,
  `recheck()`; the "Learn from me" switch (`[data-sw="learn"]`) in the Habits heading, gone from the General page.

- [ ] **Step 1: `SettingsView.habits`** (ipc.rs). Test first: in `settings_view_json_names` add `"habits"` to `expected`
  (after `"dictionary"`) and `("habits", json!([]))` to `defaults`; and

```rust
    /// The habits as the section draws them; a log shows only their count.
    #[test]
    fn settings_view_has_the_habits() {
        let mut s = Settings::default();
        let d = cue_core::habits::Draft { forms: vec![cue_core::habits::Form::Dash], app: "Word".into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        let id = s.habits.adopt(d, None, "en").id;
        let view = serde_json::to_value(SettingsView::of(&s, Lang::En, true)).unwrap();
        let expected = json!([{ "id": id, "rule": "In Word: a dash between words, not a hyphen", "before": "a - b", "after": "a — b", "app": "Word", "n": 3, "on": true }]);
        assert_eq!(view["habits"], expected);
        assert!(!format!("{:?}", SettingsView::of(&s, Lang::En, true)).contains("a — b"));
    }
```

  Run `cargo test -p cue ipc` and see it fail. Then in `SettingsView` after `learn`:

```rust
    /// The habits, the newest first, as the main window's Habits section draws them (docs/specs/2026-11-04-habits.md).
    pub habits: Vec<HabitView>,
```

```rust
/// A habit as the Habits section draws it: its rule, its example, the app it holds in (`None`: everywhere), how many
/// times it was corrected, and whether it is on.
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct HabitView {
    pub id: String,
    pub rule: String,
    pub before: String,
    pub after: String,
    pub app: Option<String>,
    pub n: u32,
    pub on: bool,
}
```

  In `of`: `habits: s.habits.list.iter().map(|h| HabitView { id: h.id.clone(), rule: h.rule.clone(), before: h.before.clone(), after: h.after.clone(), app: h.app.clone(), n: h.n, on: h.on }).collect(),`;
  in its `Debug`: `.field("habits", &self.habits.len())`. Run the tests: pass.

- [ ] **Step 2: the window's failing tests.** `windowFixture.ts`: `view()` gains `habits: [],`; add

```ts
/** A habit as the core lists it. */
export function habit(id: string, over: Partial<api.HabitView> = {}): api.HabitView {
  return { id, rule: 'In Telegram — lower case and no full stop', before: 'Send the file.', after: 'send the file', app: 'Telegram', n: 3, on: true, ...over };
}
```

  and in `stub()`'s `settingsSet`, before the generic `now = { ...now, [c.kind]: c.value };`:

```ts
    if (c.kind === 'habitOn') {
      now = { ...now, habits: now.habits.map((x) => (x.id === c.value.id ? { ...x, on: c.value.on } : x)) };
      return now;
    }
    if (c.kind === 'habitForget') {
      now = { ...now, habits: now.habits.filter((x) => x.id !== c.value) };
      return now;
    }
    if (c.kind === 'habitsForget') {
      now = { ...now, habits: [] };
      return now;
    }
```

  `entry()` gains `habits: []` in its defaults (Task 9 uses it). New `test/window-habits.test.ts`:

```ts
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { $, $$, NOW, click, core, feeds, habit, mount, settle, stub, text, view } from './windowFixture';
import type { WindowPage } from '../src/window/page';

vi.mock('../src/window/api', async (importOriginal) => {
  const real = await importOriginal<typeof import('../src/window/api')>();
  const mocked: Record<string, unknown> = { ...real };
  for (const [k, v] of Object.entries(real)) if (typeof v === 'function') mocked[k] = vi.fn();
  return mocked;
});

let page: WindowPage | null = null;
const sec = () => $('.bsec[data-scr="habits"]')!;
const rows = () => $$('.habl .hw[data-w]');
const ids = () => rows().map((r) => r.dataset.w);
const DIGITS = habit('h2', { rule: 'Numbers in digits', before: 'twelve percent', after: '12 %', app: null, n: 7 });

beforeEach(() => {
  vi.useFakeTimers({ now: NOW });
  stub(view({ habits: [habit('h1'), DIGITS] }));
});

afterEach(() => {
  page?.dispose();
  page = null;
  vi.useRealTimers();
});

describe('Habits', () => {
  it('a section after the Dictionary, with its header link, count, hint and rows', async () => {
    page = await mount();
    expect($$('.bsec').map((s) => s.dataset.scr)).toEqual(['today', 'history', 'dictionary', 'habits']);
    expect(text($$('.lk button').at(-1)!)).toBe('Habits');
    expect(text(sec().querySelector('h3 em'))).toBe('2');
    expect(text(sec().querySelector('.dhint'))).toBe(
      'How you correct text after a dictation. When a correction repeats, CUE remembers it and writes it that way itself. Kept only on this computer.',
    );
    expect(ids()).toEqual(['h1', 'h2']);
    const [first, second] = rows();
    expect(text(first.querySelector('.hrule'))).toBe('In Telegram — lower case and no full stop');
    expect(text(first.querySelector('.hex s'))).toBe('Send the file.');
    expect(text(first.querySelector('.hex ins'))).toBe('send the file');
    expect(text(first.querySelector('.hmeta'))).toBe('· in Telegram · 3 times');
    expect(text(second.querySelector('.hmeta'))).toBe('· everywhere · 7 times');
    expect(second.querySelector('.hmeta svg')).not.toBeNull();
    expect(first.querySelector('.sw')!.getAttribute('aria-checked')).toBe('true');
    expect(first.querySelector('.hdel')!.getAttribute('title')).toBe('Forget');
    expect(first.querySelector('.hdel')!.getAttribute('aria-label')).toBe('Forget: In Telegram — lower case and no full stop');
  });

  it('Russian: where and how many times', async () => {
    stub(view({ lang: 'ru', uiLanguage: 'ru', habits: [habit('h1'), habit('h2', { app: null, n: 5 })] }));
    page = await mount();
    expect(text($('.hw[data-w="h1"] .hmeta'))).toBe('· в Telegram · 3 раза');
    expect(text($('.hw[data-w="h2"] .hmeta'))).toBe('· везде · 5 раз');
    expect(text(sec().querySelector('h3'))).toContain('Привычки');
  });

  it('a switch turns a habit off in place, muted', async () => {
    page = await mount();
    const row = rows()[0];
    click(row.querySelector('.sw'));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitOn', value: { id: 'h1', on: false } });
    expect(rows()[0]).toBe(row);
    expect(row.querySelector('.hb')!.classList.contains('off')).toBe(true);
    expect(row.querySelector('.sw')!.getAttribute('aria-checked')).toBe('false');
  });

  it('"Forget" on a row folds it away and forgets the habit', async () => {
    page = await mount();
    click(rows()[0].querySelector('.hdel'));
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitForget', value: 'h1' });
    expect(ids()).toEqual(['h2']);
    expect(text(sec().querySelector('h3 em'))).toBe('1');
  });

  it('"Forget all" asks for a second press, then forgets every habit and shows the empty line', async () => {
    page = await mount();
    const b = $<HTMLButtonElement>('[data-hforget]')!;
    click(b);
    await settle();
    expect(text(b)).toBe('Press again');
    expect(core.settingsSet).not.toHaveBeenCalled();
    click(b);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'habitsForget' });
    expect(rows()).toEqual([]);
    expect($<HTMLElement>('.hempty')!.hidden).toBe(false);
    expect($<HTMLElement>('.bsec[data-scr="habits"] .bmore')!.hidden).toBe(true);
  });

  it('the first press of "Forget all" wears off after 3 s', async () => {
    page = await mount();
    const b = $('[data-hforget]')!;
    click(b);
    await vi.advanceTimersByTimeAsync(3000);
    expect(text(b)).toBe('Forget all');
    click(b);
    expect(core.settingsSet).not.toHaveBeenCalled();
  });

  it('an empty list says what will come, without "Forget all"', async () => {
    stub(view());
    page = await mount();
    expect(text($('.hempty'))).toBe('Nothing yet. CUE will notice when you correct the same thing a few times.');
    expect($<HTMLElement>('.hempty')!.hidden).toBe(false);
    expect($<HTMLElement>('.bsec[data-scr="habits"] .bmore')!.hidden).toBe(true);
    expect(text(sec().querySelector('h3 em'))).toBe('');
  });

  it('"Learn from me" sits in the heading; off, the rows are muted', async () => {
    page = await mount();
    const sw = $<HTMLButtonElement>('.bsec[data-scr="habits"] h3 [data-sw="learn"]')!;
    expect(sw.getAttribute('role')).toBe('switch');
    expect(sw.getAttribute('aria-checked')).toBe('true');
    click(sw);
    await settle();
    expect(core.settingsSet).toHaveBeenCalledWith({ kind: 'learn', value: false });
    expect(sw.getAttribute('aria-checked')).toBe('false');
    expect($('.habl')!.classList.contains('paused')).toBe(true);
  });

  it('settings the core sends: a new habit grows in with the highlight, one undone from the card goes', async () => {
    page = await mount();
    feeds.settings(view({ habits: [habit('h3', { rule: 'No full stop' }), habit('h1'), DIGITS] }));
    expect(ids()).toEqual(['h3', 'h1', 'h2']);
    expect(rows()[0].querySelector('.hb')!.classList.contains('new')).toBe(true);
    expect(rows()[1].querySelector('.hb')!.classList.contains('new')).toBe(false);
    feeds.settings(view({ habits: [habit('h1'), DIGITS] }));
    expect(ids()).toEqual(['h1', 'h2']);
  });

  it('with no cleanup connected, a line says habits need it', async () => {
    page = await mount();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(false);
    expect(text($('.hnocl'))).toBe('Habits work when the cleanup is on.');
  });

  it('a key saved: no such line', async () => {
    core.keyState.mockResolvedValue({ saved: true, mask: 'sk-…789' });
    page = await mount();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(true);
  });

  it('a local server needs no key: no such line', async () => {
    stub(view({ refineConnection: 'ollama', refineModel: 'gemma4:12b', refineAddress: 'http://localhost:11434/v1' }));
    page = await mount();
    expect($<HTMLElement>('.hnocl')!.hidden).toBe(true);
  });
});
```

  In `window-settings.test.ts` replace `"Learn from me" switch sends
  the learn change` (line 787) with:

```ts
  it('the General page has "Start at sign-in" only: "Learn from me" lives in the Habits heading', async () => {
    page = await mount();
    await open();
    await sub('general');
    expect($('.btray [data-sw="learn"]')).toBeNull();
    expect($('.btray [data-sw="autostart"]')).not.toBeNull();
  });
```

  and in "settings pushed by the core leave the tray as it is…" (line 800) rename it "…: a draft stays, the Habits
  switch follows" and replace its last six lines (from `key('Escape');`) with:

```ts
    feeds.settings(view({ refineStyle: 'custom', learn: false }));
    expect($('.bsec[data-scr="habits"] [data-sw="learn"]')!.getAttribute('aria-checked')).toBe('false');
    expect($('.btray textarea.cta')).toBe(field);
```

  Run `npx vitest run test/window-habits.test.ts test/window-settings.test.ts` and see them fail.

- [ ] **Step 3: `api.ts`.** After `SettingsView`'s `learn`: `/** The habits, the newest first (docs/specs/2026-11-04-habits.md). */ habits: HabitView[];`
  and above `SettingsView`:

```ts
/** A habit as the Habits section draws it: its rule, its example, the app it holds in (null — everywhere), how many
 *  times it was corrected, and whether it is on. */
export interface HabitView {
  id: string;
  rule: string;
  before: string;
  after: string;
  app: string | null;
  n: number;
  on: boolean;
}
```

  `SettingChange` gains `| { kind: 'habitOn'; value: { id: string; on: boolean } } | { kind: 'habitForget'; value: string } | { kind: 'habitsForget' }`.

- [ ] **Step 4: `text.ts`.** In `WinText`, after `dictRemoveWord`, with doc comments; remove `learnHint` (interface, en,
  ru: the General row is gone); keep `learn`:

```ts
  /** The Habits section (docs/specs/2026-11-04-habits.md): its heading (also the header's link), the hint, the line
   *  when no cleanup is connected, the empty list, "Forget all" and its second press, a row's forget button (its name
   *  for a screen reader: {NAME} — the rule), its switch's tooltips, where it holds ({NAME} — the app), how many times,
   *  and the History's chips for a screen reader. */
  habits: string;
  habitsHint: string;
  habitsNoCleanup: string;
  habitsEmpty: string;
  habitsForgetAll: string;
  habitsForgetAgain: string;
  habitForget: string;
  habitForgetName: string;
  habitOff: string;
  habitOn: string;
  habitEverywhere: string;
  habitIn: string;
  habitTimes: Forms;
  habitsApplied: string;
```

  `en`:

```ts
  habits: 'Habits',
  habitsHint: 'How you correct text after a dictation. When a correction repeats, CUE remembers it and writes it that way itself. Kept only on this computer.',
  habitsNoCleanup: 'Habits work when the cleanup is on.',
  habitsEmpty: 'Nothing yet. CUE will notice when you correct the same thing a few times.',
  habitsForgetAll: 'Forget all',
  habitsForgetAgain: 'Press again',
  habitForget: 'Forget',
  habitForgetName: 'Forget: {NAME}',
  habitOff: 'Turn off',
  habitOn: 'Turn on',
  habitEverywhere: 'everywhere',
  habitIn: 'in {NAME}',
  habitTimes: ['time', 'times'],
  habitsApplied: 'Habits applied',
```

  `ru`:

```ts
  habits: 'Привычки',
  habitsHint: 'Как вы правите текст после диктовки. Когда правка повторяется, CUE запоминает её и дальше пишет так сам. Хранится только на этом компьютере.',
  habitsNoCleanup: 'Привычки работают, когда включена очистка.',
  habitsEmpty: 'Пока ничего. CUE заметит, если вы несколько раз поправите одно и то же.',
  habitsForgetAll: 'Забыть всё',
  habitsForgetAgain: 'Нажмите ещё раз',
  habitForget: 'Забыть',
  habitForgetName: 'Забыть: {NAME}',
  habitOff: 'Выключить',
  habitOn: 'Включить',
  habitEverywhere: 'везде',
  habitIn: 'в {NAME}',
  habitTimes: ['раз', 'раза', 'раз'],
  habitsApplied: 'Применённые привычки',
```

- [ ] **Step 5: `icons.ts`.** In `OWN` after `pause`, the trash of design stage 5 on the same grid:
  `trash: SV('<path d="M4 5.8h12M8 5.8V4h4v1.8M5.6 5.8l.7 10.4h7.4l.7-10.4"/>'),`; `IconName` gains `| 'habit'` (the
  shared set's); the header comment adds "the trash of a habit's row".

- [ ] **Step 6: `habits.ts`** (new):

```ts
// The Habits section (docs/specs/2026-11-04-habits.md; design stage 5, variant A): how the user corrects text after a
// dictation, learned when a correction repeats. The heading with the count and the "Learn from me" switch; the hint;
// one row per habit, the newest first — the rule, its example struck → green, where it holds and how many times, a
// switch, and "Forget" on hover; "Forget all" at the bottom on a second press. With "Learn from me" off the rows are
// muted; with no cleanup connected a line says habits need it. The core keeps the habits (`habits.list` in
// settings.json) and changes them (`habitOn`, `habitForget`, `habitsForget`); the rows follow its answers and what it
// sends by itself: a new one grows in with the highlight, a gone one folds away, one switched off is muted in place.
// Text goes into the page as text.
import { CUE } from '../designParts';
import * as api from './api';
import type { HabitView } from './api';
import { icon } from './icons';
import { h, iconButton, logFailed, type Ctx } from './page';
import { CLEAR_ARMED_MS } from './settings';
import { fill, plural } from './text';

/** How long a row takes to grow in or fold away (window.css: .hw.anim). */
const ROW_MS = 450;

export class HabitsView {
  private sec: HTMLElement | null = null;
  private count!: HTMLElement;
  private learn!: HTMLButtonElement;
  private list!: HTMLElement;
  private empty!: HTMLElement;
  private noCleanup!: HTMLElement;
  private more!: HTMLElement;
  private forgetAll!: HTMLButtonElement;
  private armed: ReturnType<typeof setTimeout> | null = null;
  /** The habits drawn: one not among them comes in with the highlight. */
  private drawn = new Set<string>();
  /** The connection the no-cleanup line was checked for. */
  private checked: string | null = null;

  constructor(private readonly ctx: Ctx) {}

  private get habits(): HabitView[] {
    return this.ctx.settings?.habits ?? [];
  }

  private get learning(): boolean {
    return this.ctx.settings?.learn ?? true;
  }

  /** Draws the section into `sec` (the page builds it again on a language change). */
  mount(sec: HTMLElement): void {
    const t = this.ctx.t;
    this.sec = sec;
    sec.textContent = '';
    this.disarm();
    const h3 = h('h3', '', t.habits);
    this.count = h('em', 'lbl');
    const right = h('span', 'h3r', t.learn);
    this.learn = h('button', 'sw');
    this.learn.type = 'button';
    this.learn.dataset.sw = 'learn';
    this.learn.setAttribute('role', 'switch');
    this.learn.setAttribute('aria-label', t.learn);
    this.learn.addEventListener('click', () => void this.ctx.change({ kind: 'learn', value: !this.learning }));
    right.append(this.learn);
    h3.append(this.count, right);
    this.noCleanup = h('p', 'hnocl', t.habitsNoCleanup);
    this.noCleanup.hidden = true;
    this.list = h('div', 'habl');
    this.list.setAttribute('role', 'list');
    this.list.addEventListener('click', (e) => this.onClick(e));
    this.empty = h('p', 'hempty', t.habitsEmpty);
    this.more = h('div', 'bmore');
    this.forgetAll = h('button');
    this.forgetAll.type = 'button';
    this.forgetAll.dataset.hforget = '';
    this.forgetAll.append(h('span', 'lbl', t.habitsForgetAll));
    this.forgetAll.addEventListener('click', () => this.forgetAllPressed());
    this.more.append(this.forgetAll);
    sec.append(h3, h('p', 'dhint', t.habitsHint), this.noCleanup, this.list, this.empty, this.more);
    this.drawn.clear();
    this.checked = null;
    this.render(false);
  }

  /** The habits as the core holds them now: rows kept are changed in place, new ones grow in, gone ones fold away. */
  render(animate = true): void {
    if (!this.sec) return;
    const list = this.habits;
    const n = list.length ? String(list.length) : '';
    if (animate) CUE().setLabel(this.count, n);
    else {
      this.count.textContent = n;
      this.count.dataset.t = n;
    }
    this.learn.setAttribute('aria-checked', String(this.learning));
    this.list.classList.toggle('paused', !this.learning);
    const rows = new Map([...this.list.querySelectorAll<HTMLElement>('.hw[data-w]')].map((w) => [w.dataset.w!, w]));
    const keep = new Set(list.map((x) => x.id));
    for (const [id, w] of rows) if (!keep.has(id)) this.fold(w, animate);
    const fresh: HTMLElement[] = [];
    let prev: HTMLElement | null = null;
    for (const x of list) {
      let w = rows.get(x.id);
      if (w) this.update(w, x);
      else {
        w = this.row(x);
        if (this.drawn.size || animate) fresh.push(w);
      }
      if (prev) {
        if (prev.nextElementSibling !== w) prev.after(w);
      } else if (this.list.firstElementChild !== w) this.list.prepend(w);
      prev = w;
    }
    for (const w of fresh) this.grow(w);
    this.drawn = keep;
    this.empty.hidden = list.length > 0;
    this.more.hidden = list.length === 0;
    const conn = this.ctx.settings?.refineConnection ?? null;
    if (conn !== this.checked) {
      this.checked = conn;
      void this.recheck();
    }
  }

  /** The line "Habits work when the cleanup is on": shown while the cleanup's connection is a cloud one with no key
   *  saved (a local server needs none). Asked again when the window gets the focus back. */
  async recheck(): Promise<void> {
    if (!this.sec) return;
    const s = this.ctx.settings;
    const c = s?.connections.find((x) => x.id === s.refineConnection);
    if (!c || c.local) {
      this.noCleanup.hidden = true;
      return;
    }
    try {
      this.noCleanup.hidden = (await api.keyState(c.id as api.KeyName)).saved;
    } catch (e) {
      logFailed('reading the cleanup key', e);
      this.noCleanup.hidden = true;
    }
  }

  private row(x: HabitView): HTMLElement {
    const t = this.ctx.t;
    const w = h('div', 'hw');
    w.dataset.w = x.id;
    const b = h('div', 'hb');
    b.setAttribute('role', 'listitem');
    const bx = h('div', 'hbx');
    bx.append(h('b', 'hrule'), h('div', 'hex'));
    const del = iconButton('ibt sm hdel', 'trash', t.habitForget);
    del.dataset.forget = x.id;
    const sw = h('button', 'sw');
    sw.type = 'button';
    sw.dataset.hsw = x.id;
    sw.setAttribute('role', 'switch');
    b.append(bx, del, sw);
    w.append(b);
    this.update(w, x);
    return w;
  }

  /** A row says what the habit is now: its rule, example, where, how many times, on or off. */
  private update(w: HTMLElement, x: HabitView): void {
    const t = this.ctx.t;
    const b = w.firstElementChild as HTMLElement;
    b.classList.toggle('off', !x.on);
    b.querySelector('.hrule')!.textContent = x.rule;
    const ex = b.querySelector('.hex')!;
    ex.textContent = '';
    const where = h('span', 'hmeta', '· ');
    if (x.app) where.append(fill(t.habitIn, x.app));
    else {
      where.insertAdjacentHTML('beforeend', icon('globe'));
      where.append(t.habitEverywhere);
    }
    where.append(` · ${x.n}\u00a0${plural(x.n, t.habitTimes)}`);
    ex.append(h('s', '', x.before), ' → ', h('ins', '', x.after), where);
    b.querySelector('.hdel')!.setAttribute('aria-label', fill(t.habitForgetName, x.rule));
    const sw = b.querySelector<HTMLElement>('.sw')!;
    sw.setAttribute('aria-checked', String(x.on));
    sw.setAttribute('aria-label', x.rule);
    sw.title = x.on ? t.habitOff : t.habitOn;
  }

  private onClick(e: Event): void {
    const el = e.target as Element;
    const sw = el.closest<HTMLElement>('[data-hsw]');
    if (sw) {
      const id = sw.dataset.hsw!;
      const x = this.habits.find((y) => y.id === id);
      if (x) void this.ctx.change({ kind: 'habitOn', value: { id, on: !x.on } });
      return;
    }
    const del = el.closest<HTMLElement>('[data-forget]');
    if (del) void this.forget(del.dataset.forget!);
  }

  /** The row folds away, then the core forgets the habit; a refusal draws it back. */
  private async forget(id: string): Promise<void> {
    const w = this.list.querySelector<HTMLElement>(`.hw[data-w="${id}"]`);
    if (w) this.fold(w, true);
    await new Promise((r) => setTimeout(r, this.ctx.reduced() ? 0 : ROW_MS));
    if (!(await this.ctx.change({ kind: 'habitForget', value: id }))) this.render(false);
  }

  /** "Forget all" works on a second press within CLEAR_ARMED_MS, as "Clear history" does. */
  private forgetAllPressed(): void {
    const t = this.ctx.t;
    const l = this.forgetAll.querySelector<HTMLElement>('.lbl')!;
    if (!this.armed) {
      this.armed = setTimeout(() => {
        this.armed = null;
        this.forgetAll.classList.remove('armed');
        if (l.isConnected) CUE().setLabel(l, t.habitsForgetAll);
      }, CLEAR_ARMED_MS);
      this.forgetAll.classList.add('armed');
      CUE().setLabel(l, t.habitsForgetAgain);
      return;
    }
    this.disarm();
    this.forgetAll.classList.remove('armed');
    CUE().setLabel(l, t.habitsForgetAll);
    void this.ctx.change({ kind: 'habitsForget' });
  }

  private disarm(): void {
    if (this.armed) clearTimeout(this.armed);
    this.armed = null;
  }

  /** A new row grows from nothing with the highlight of a new dictionary chip. */
  private grow(w: HTMLElement): void {
    (w.firstElementChild as HTMLElement).classList.add('new');
    if (this.ctx.reduced()) return;
    w.classList.add('anim', 'shut');
    void w.offsetHeight;
    w.classList.remove('shut');
    w.style.height = `${w.scrollHeight}px`;
    setTimeout(() => {
      w.classList.remove('anim');
      w.style.height = '';
    }, ROW_MS);
  }

  /** A gone row folds away (at once with reduced motion or when nothing moves). */
  private fold(w: HTMLElement, animate: boolean): void {
    delete w.dataset.w;
    if (!animate || this.ctx.reduced()) {
      w.remove();
      return;
    }
    w.style.height = `${w.offsetHeight}px`;
    w.classList.add('anim');
    void w.offsetHeight;
    w.classList.add('shut');
    setTimeout(() => w.remove(), ROW_MS);
  }
}
```

- [ ] **Step 7: `page.ts`.** Import `HabitsView`; `readonly habits: HabitsView;` with the others and
  `this.habits = new HabitsView(this);` after `this.dict = …`. In `build()`: `const habitsSec = section('habits', t.habits);`,
  `this.sections = [todaySec, sec, dictSec, habitsSec];`, the links gain `['habits', t.habits]`,
  `col.append(todaySec, sec, dictSec, habitsSec);`, and after `this.dict.mount(dictSec);` `this.habits.mount(habitsSec);`.
  In `took()`'s else branch, after `this.dict.render();`: `this.habits.render();`. `pushed()` becomes:

```ts
  /** Settings the core sent by itself (a word learned or taken back from the Drop, a habit learned, widened or undone
   *  from its card): the Dictionary and the Habits are drawn again. The tray is not redrawn: a draft in it (the
   *  instructions are saved when their field loses focus) stays as typed. */
  private pushed(view: SettingsView): void {
    this.settings = view;
    this.dict.render();
    this.habits.render();
  }
```

  In the window's `focus` listener (constructor): `void this.habits.recheck();`. The header comment's first sentence
  gains "the Dictionary and Habits sections".

- [ ] **Step 8: `settings.ts`.** `generalRows()` (line 803) returns `[srow(t.autostart, null, sw).row]` (the `learn`
  switch goes); the `[data-sw="learn"]` branch of `onClick` (line 465) goes; `learnChanged()` (line 298) goes.

- [ ] **Step 9: `window.css`**, after the Dictionary's rules (`.dch .g`), from the approved variant A:

```css
/* The Habits section (docs/specs/2026-11-04-habits.md, design stage 5, variant A): the switch at the heading's right,
   a row per habit — the rule, its example struck → green, where and how many times, "Forget" on hover and a switch. Off
   (a habit, or "Learn from me"), the words take --muted: never opacity, so they keep their contrast. */
.bsec>h3 .h3r{order:6;display:flex;align-items:center;gap:8px;font-size:12px;font-weight:400;color:var(--muted)}
.bsec>h3 em.lbl{display:inline-flex}
.hnocl{margin:-4px 0 10px;font-size:12px;color:var(--warn-ink)}
.habl{display:flex;flex-direction:column;margin:0 -8px}
.hw.anim{overflow:hidden;transition:height .45s var(--e-sm),opacity .3s}
.hw.shut{height:0!important;opacity:0}
.hb{position:relative;display:grid;grid-template-columns:minmax(0,1fr) auto auto;column-gap:10px;align-items:center;padding:8px;border-radius:8px;transition:background .2s}
.hb:hover{background:color-mix(in srgb,var(--text) 4%,transparent)}
.hb.new{animation:rowIn .6s var(--e-spring),habHl 2.8s var(--e-out)}
@keyframes habHl{0%,30%{background:color-mix(in srgb,var(--ok) 14%,transparent)}}
.hbx{min-width:0}
.hrule{display:block;font-size:13px;font-weight:500;line-height:20px;color:var(--text);transition:color .25s}
.hex{font-size:12px;line-height:18px;color:var(--muted)}
.hex s{text-decoration-thickness:1px}
.hex ins{-webkit-box-decoration-break:clone;box-decoration-break:clone;text-decoration:none;color:var(--text);background:color-mix(in srgb,var(--ok) 15%,transparent);border-radius:3px;padding:0 3px;transition:background .25s,color .25s}
.hmeta{margin-left:6px;white-space:nowrap}
.app .hmeta svg{display:inline-block;width:12px;height:12px;vertical-align:-2px;margin-right:3px}
.hdel{opacity:0;transition:opacity .2s}
.hb:hover .hdel,.hdel:focus-visible{opacity:1}
.hb.off .hrule,.habl.paused .hrule{color:var(--muted)}
.hb.off ins,.habl.paused ins{background:color-mix(in srgb,var(--text) 6%,transparent);color:var(--muted)}
.hempty{margin:0;font-size:12px;color:var(--muted);animation:msIn .3s var(--e-out) both}
.bmore button.armed{color:var(--warn-ink)}
@media (prefers-reduced-motion:reduce){.hb.new{animation:none}.hw.anim{transition:none}.hempty{animation:none}}
```

  Add `.hdel` to the `:focus-visible` outline list at line 429 only if `.ibt` is not already in it.

- [ ] **Step 10: run** `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`, `cargo test -p cue`: green.

---

### Task 9: the History's chips

**Files:**
- Modify: `app/src-tauri/src/ipc.rs` (`EntryView`, line 171), `app/ui/src/window/api.ts` (`EntryView`, line 16),
  `app/ui/src/window/history.ts` (`details`, line 532), `app/ui/src/design/window.css`
- Test: `app/src-tauri/src/ipc.rs` tests, `app/ui/test/window-history.test.ts`

**Interfaces:**
- Consumes: `history::Entry.habits` (Task 2), filled by the runtime (Task 6).
- Produces: `EntryView.habits: Vec<history::HabitMark>` (JSON `habits: [{ id, short }]`, always present); TS
  `EntryView.habits: { id: string; short: string }[]`.

- [ ] **Step 1: the failing tests.** In `ipc.rs`'s `entry_view_has_the_time_of_day` add `"habits": [],` to the expected
  JSON; and

```rust
    /// The habits that shaped a dictation go to the window with it, by their short names; a log shows their count.
    #[test]
    fn entry_view_has_the_habits_that_shaped_it() {
        let e = history::Entry {
            id: "2026-11-04T10:00:00.000-1".into(),
            ts: "2026-11-04T10:00:00.000".into(),
            mode: history::Mode::Dictation,
            app: "Telegram Desktop".into(),
            text: "скинь файл".into(),
            question: None,
            raw: Some("Скинь файл".into()),
            habits: vec![history::HabitMark { id: "h1".into(), short: "без точки".into() }],
        };
        let view = serde_json::to_value(EntryView::of(&e, false)).unwrap();
        assert_eq!(view["habits"], json!([{ "id": "h1", "short": "без точки" }]));
        assert!(!format!("{:?}", EntryView::of(&e, false)).contains("точки"));
    }
```

  In `window-history.test.ts`, in "How it was said":

```ts
  it('an entry a habit shaped lists it under how it was said, by its short name', async () => {
    const shaped = entry(`${TODAY}T11:00:00.000-1`, 'send the file', { app: 'Telegram', raw: 'Send the file.', habits: [{ id: 'h1', short: 'no full stop' }] });
    core.historyPage.mockResolvedValue(pageOf(shaped, said));
    page = await mount();
    openRow(shaped.id);
    const box = rowOf(shaped.id).querySelector('.hdet .hhabs')!;
    expect(box.getAttribute('aria-label')).toBe('Habits applied');
    const chips = [...box.querySelectorAll('.hhab')];
    expect(chips.map((c) => text(c))).toEqual(['no full stop']);
    expect(chips[0].querySelector('svg')).not.toBeNull();
    openRow(said.id);
    expect(rowOf(said.id).querySelector('.hdet .hhabs')).toBeNull();
  });
```

  Run `cargo test -p cue ipc` and `npx vitest run test/window-history.test.ts`: they fail.

- [ ] **Step 2: implement.** `EntryView` gains `pub habits: Vec<history::HabitMark>,` (doc: "…and the habits whose
  effect showed in its text, by their short names"), `of` sets `habits: e.habits.clone()`, its `Debug` adds
  `.field("habits", &self.habits.len())`. TS `EntryView` gains
  `/** The habits whose effect showed in its text, by their short names as they were then. */ habits: { id: string; short: string }[];`.
  In `history.ts`'s `details`, before `inner.append(this.player(e).el);`:

```ts
    if (e.habits?.length) inner.append(this.habitChips(e));
```

  and the method after `details`:

```ts
  /** The habits that shaped the entry's text (docs/specs/2026-11-04-habits.md): a chip each, the habit icon and its
   *  short name as it was then (a forgotten habit keeps its chip). */
  private habitChips(e: EntryView): HTMLElement {
    const box = h('div', 'hhabs');
    box.setAttribute('role', 'list');
    box.setAttribute('aria-label', this.ctx.t.habitsApplied);
    for (const m of e.habits) {
      const chip = h('span', 'hhab');
      chip.setAttribute('role', 'listitem');
      chip.innerHTML = icon('habit');
      chip.append(m.short);
      box.append(chip);
    }
    return box;
  }
```

  (`icon` is imported in `history.ts`; if not, import it from `./icons`). `window.css`, next to `.hraw`:

```css
/* The habits that shaped an entry: small chips under how it was said. */
.hhabs{display:flex;flex-wrap:wrap;gap:6px;margin-top:8px}
.hhab{display:inline-flex;align-items:center;gap:4px;height:22px;padding:0 8px;border-radius:999px;font-size:11px;line-height:1;color:var(--text);background:color-mix(in srgb,var(--ok) 12%,transparent)}
.hhab svg{width:12px;height:12px;color:var(--ok)}
```

- [ ] **Step 3: run** `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`, `cargo test --workspace`: green.

---

### Task 10: documents, the checks, the live check

**Files:**
- Modify: `README.md`, `docs/ARCHITECTURE.md`

- [ ] **Step 1: README.**
  - Privacy (line 85), after the "Your corrections stay on this computer." bullet, a sentence in it: "When you correct
    the same thing a few times, CUE keeps it as a habit: the words around the correction (at most 40 characters before
    and after it) are the only text of a correction it writes to disk, and the cleanup's connection receives them once,
    with the kind of the correction and the app's name, to word the habit's rule; it already received that text when
    you dictated it."
  - The settings list (line 255): "**General** (Start at sign-in)"; the switch "**Learn from me**" is now "in the Habits
    section's heading".
  - After "Words from your corrections" (line 257), a new bullet **Habits**: the three corrections in one app (the end
    mark, the first letter's case, a word written another way, numbers as digits, a dash for a hyphen), the "New habit"
    card with **Undo** (7 s, hovering pauses it), the habit going into the cleanup of that app's dictations (Translate
    and "Ask" take none), widening to every app when it repeats in another, the main window's Habits section (switch,
    Forget on hover, Forget all on a second press, muted while "Learn from me" is off, the line when no cleanup is on),
    and the History's chips.
  - The settings.json part (after `learn.enabled`, line 506): `habits.seen` (counts by hash, no text, 30 days, at most
    300), `habits.list` (`{ id, forms, rule, short, before, after, app, n, on }`, newest first, at most 50),
    `habits.declined` (hashes of forms never to learn again); each list written only when not empty, anything that does
    not read is skipped. `learn.enabled` `false` also stops habits from being counted, learned and applied.
  - Privacy in detail (line 706, "Learn from me"): what the habit keeps on disk (the example, the rule, the short name;
    counts by hash), what the rule request sends and when, that it is not sent without a cleanup connection (the rule
    then comes from a template), and that the log gets "habit learned" with kinds and numbers.
- [ ] **Step 2: ARCHITECTURE.** After "## Corrections", a "## Habits" section: `CoreEvent::Corrected { …, app }` →
  `habits::correction` (the kinds over `corrections::runs`, the forms and their FNV-1a hashes) → `Habits::count`
  (`seen` by app and hash, `LEARN_AT`, widening, `declined`) → `Draft` → `cue_engines::habit::rule` through the
  cleanup's connection (`habits::rule_messages`: kind, example, app; `RULE_WAIT_MS`, else `habits::template`) →
  `CoreEvent::HabitRule` → `Habits::adopt` → `cue://settings`; the card queue (`habit_cards`, `first_notice` with the
  suggestion's `offer_found`, `at_rest`) → `cue://habit-rule` + `cue://tray` `newHabit` → `trayAction` →
  `SettingChange::HabitForget`; and the request side: `Habits::for_app` → `RefineContext::with_habits` →
  `refine::habits_prompt` after the cache mark (Clean and Rewrite only) → `habits::marks` on a refined text →
  `Entry.habits` → the History's chips.
- [ ] **Step 3: all checks** from the global constraints: green. The leak scan (`cargo run -p cue-cli -- leak-scan …`,
  as `scripts/build-release.ps1 -ScanBuild` runs it) still passes on a settings.json with a habit.
- [ ] **Step 4: live, with the owner** (build the app the usual way, `npx tauri dev` from `app` or the release build;
  cleanup on Anthropic with a key):
  1. Telegram: dictate "Скинь файл", change it to "скинь файл" (lower case, no full stop) three times: after the third
     the card "Новая привычка" with the rule; it closes after 7 s; hovering pauses it. The next dictation in Telegram
     comes without the capital and the full stop; its History entry shows the chip «без точки».
  2. "Отменить" on a new card: «Убрано», the row leaves the Habits section, the same correction is never learned again.
  3. Почта: "Здравствуйте, …" → "Добрый день, …" three times: a replacement habit; its rule names both.
  4. Документы: "клиенты - в основном" → "клиенты — в основном" three times: a dash habit; then three times in another
     app: the row says «везде», no card.
  5. With no key (or the network off): the rule comes from the template within 5 s.
  6. "Learn from me" off in the Habits heading: rows muted, nothing learned, the next dictation's request has no habits
     (the log has no "habit forms counted").
  7. A dictionary suggestion and a habit from one correction: the capsule first, the card after it folds.
  8. Both themes; reduced motion.
- [ ] **Step 5: report to the owner** what passed, what did not, and ask whether to commit.

---

### Task 11: the hash's secret (`habits::Secret`, Credential Manager, a new one resets the counts)

Decided 06.11.2026 (spec, "The hash's secret"). Line numbers are those of the working tree after Tasks 1–10.

**Files:**
- Modify: `Cargo.toml` (workspace dependencies, after `sha2 = "0.10"`, line 42), `app/crates/core/Cargo.toml`,
  `app/crates/engines/Cargo.toml`
- Modify: `app/crates/core/src/habits.rs`, `app/crates/core/src/settings.rs` (test, line 2201)
- Modify: `app/crates/engines/src/keys.rs`, `app/src-tauri/src/runtime.rs` (`run`, line 472), `app/crates/cli/src/leak.rs`
  (`gather`, line 186)
- Modify: `README.md`, `docs/ARCHITECTURE.md`

**Interfaces:**
- Consumes: Tasks 1–10 as built.
- Produces in `habits`: `pub const SECRET_BYTES: usize = 32`; `pub struct Secret([u8; 32])` (Clone, PartialEq, Eq,
  Default = all zero = no secret; hand-written `Debug`: `Secret(..)` or `Secret(none)`, nothing of the bytes) with
  `from_bytes([u8; 32])`, `is_set()`, `to_hex() -> String` (64 hex digits, for the key store only),
  `from_hex(&str) -> Option<Secret>`; `Form::hash(&self, secret: &Secret) -> String` (pure: HMAC-SHA-256 of the kind and
  detail under the secret, the first 8 bytes as 16 hex digits); `Habit::hashes(&self, secret: &Secret)`;
  `Habits.secret: Secret` (`#[serde(skip)]`: never in settings.json); `Habits::rekey(&mut self, Secret) -> (usize, usize)`
  (the counts and declined forms dropped); `id_of(forms: &[Form], secret: &Secret) -> String`. `Habits::is_default`
  ignores the secret, `forget_all` keeps it, and `count` counts nothing without one.
- Produces in `keys`: `pub enum SecretRead { Kept(Secret), Made { secret: Secret, saved: Result<(), EngineError> } }`,
  `pub fn habit_secret() -> Result<SecretRead, EngineError>` (reads, or makes and saves a new one; `Err` only without
  random bytes), `pub fn kept_habit_secret() -> Option<Secret>` (reads, never makes); the entry `habits-secret` under
  `SERVICE` ("CUE").

- [ ] **Step 1: write the failing tests.** In `habits.rs`'s `mod tests`, after `const STOP`:

```rust
    fn key(byte: u8) -> Secret {
        Secret::from_bytes([byte; SECRET_BYTES])
    }
    /// The habits with a secret, as the runtime holds them.
    fn habits() -> Habits {
        Habits { secret: key(7), ..Habits::default() }
    }
```

  Replace `the_hash_is_stable` (line 839) with:

```rust
    /// HMAC-SHA-256 under the secret: the same form and secret give the same hash in every version, so a count
    /// survives an update; another secret gives another hash.
    #[test]
    fn the_hash_is_stable_under_one_secret_and_another_under_another() {
        let k = key(1);
        assert_eq!(Form::Digits.hash(&k), "0cda9c2120dc4085");
        assert_eq!(Form::Dash.hash(&k), "1e280b5650f82678");
        assert_eq!(STOP.hash(&k), "d61bb7341f7ef877");
        assert_eq!(Form::FirstLetter { lower: true }.hash(&k), "1465766befda9e4e");
        assert_eq!(Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() }.hash(&k), "06f5620c5fa0f9db");
        assert_eq!(Form::Digits.hash(&k), Form::Digits.hash(&key(1)), "the same form and secret");
        assert_ne!(Form::EndMark { mark: '.', removed: false }.hash(&k), STOP.hash(&k));
        for form in [Form::Digits, Form::Dash, STOP] {
            assert_ne!(form.hash(&k), form.hash(&key(2)), "{form:?} under another secret");
        }
        assert_eq!(Form::Digits.hash(&key(2)), "d74d04473faae824");
    }

    /// RFC 4231, test case 2.
    #[test]
    fn hmac_sha256_is_the_standard_one() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn the_secret_reads_back_from_its_hex_digits_and_shows_nothing() {
        let k = key(0x5c);
        assert_eq!(k.to_hex(), "5c".repeat(SECRET_BYTES));
        assert_eq!(Secret::from_hex(&k.to_hex()), Some(k.clone()));
        let (short, long, letters, signs, zero) =
            ("5c".to_owned(), "5c".repeat(SECRET_BYTES + 1), "zz".repeat(SECRET_BYTES), "+5".repeat(SECRET_BYTES), "00".repeat(SECRET_BYTES));
        for bad in ["", &short, &long, &letters, &signs, &zero] {
            assert_eq!(Secret::from_hex(bad), None, "{bad}");
        }
        assert_eq!(format!("{k:?}"), "Secret(..)");
        assert_eq!(format!("{:?}", Secret::default()), "Secret(none)");
        let h = Habits { secret: k, ..Habits::default() };
        let shown = format!("{h:?}");
        assert!(!shown.contains("5c5c") && !shown.contains("92, 92"), "{shown}");
        assert_eq!(serde_json::to_value(&h).unwrap(), serde_json::json!({}), "never written");
    }
```

  After `three_times_in_one_app_make_one_habit_of_the_forms_together`:

```rust
    #[test]
    fn without_a_secret_nothing_is_counted() {
        let mut h = Habits { secret: Secret::default(), ..Habits::default() };
        let c = correction("Скинь файл.", "скинь файл");
        for _ in 0..LEARN_AT {
            assert_eq!(h.count(TG, &c, "2026-11-04", "en"), Counted::default());
        }
        assert!(h.seen.is_empty());
    }

    /// The secret was missing or unreadable, and a new one was made: the counts and the declined forms go (their hashes
    /// no longer match), the habits keep working with their ids made again.
    #[test]
    fn a_new_secret_drops_the_counts_and_the_declined_forms_and_keeps_the_habits() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count(TG, &correction("клиенты - в основном", "клиенты — в основном"), "2026-11-04", "en");
        h.declined.push(Form::Digits.hash(&h.secret));
        assert_eq!(h.rekey(key(9)), (1, 1));
        assert!(h.seen.is_empty() && h.declined.is_empty());
        assert_eq!(h.secret, key(9));
        let id = id_of(&[Form::FirstLetter { lower: true }, STOP], &key(9));
        assert_ne!(id, old);
        assert_eq!(h.list[0].id, id, "its id is made with the new secret");
        assert_eq!(h.for_app(TG).count(), 1, "still applied");
        assert!(h.count(TG, &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en").grown, "and counted on");
        assert!(h.forget(&id));
        assert_eq!(h.declined, [Form::FirstLetter { lower: true }.hash(&key(9)), STOP.hash(&key(9))]);
    }
```

  The existing tests take the secret (the counting ones need it; nothing else in them changes):
  - every `let mut h = Habits::default();` (twelve of them) becomes `let mut h = habits();`;
  - `counts_older_than_thirty_days_go_and_at_most_three_hundred_stay`: `let mut h = habits();` moves up, and the first
    line becomes `let (dash, digits) = (Form::Dash.hash(&h.secret), Form::Digits.hash(&h.secret));`;
  - `a_forgotten_habit_is_never_learned_again_until_all_is_forgotten`: `[Form::FirstLetter { lower: true }.hash(&h.secret), STOP.hash(&h.secret)]`,
    and `assert_eq!(h, Habits::default());` becomes `assert_eq!(h, habits(), "the secret stays");`;
  - `three_times_in_one_app_make_one_habit_of_the_forms_together`: `assert_eq!(habit.id, "hca6cdd4168133ac4");`
    (the FNV-1a of its forms' hashes under `key(7)`).

  In `settings.rs`, line 2201: `assert_eq!(s.habits.declined, [crate::habits::Form::Dash.hash(&s.habits.secret)]);`.

  In `keys.rs`'s tests, after `write_failure_is_service_error`:

```rust
    /// The habits' secret as the store gives it back; missing, unreadable or not a secret, it is none and a new one is
    /// made. Nothing of a store error goes anywhere.
    #[test]
    fn a_kept_secret_reads_back_and_anything_else_is_none() {
        let secret = Secret::from_bytes([0x5c; 32]);
        assert_eq!(kept_secret(Ok(secret.to_hex())), Some(secret.clone()));
        assert_eq!(kept_secret(Ok(format!(" {}\r\n", secret.to_hex()))), Some(secret));
        assert_eq!(kept_secret(Err(keyring::Error::NoEntry)), None);
        assert_eq!(kept_secret(Err(platform_failure("boom"))), None);
        assert_eq!(kept_secret(Err(keyring::Error::BadEncoding(b"5c5c".to_vec()))), None);
        assert_eq!(kept_secret(Ok("not a secret".into())), None);
    }

    #[test]
    fn a_new_secret_is_random_and_shows_nothing() {
        let (a, b) = (new_secret().unwrap(), new_secret().unwrap());
        assert!(a.is_set() && a != b);
        assert_eq!(format!("{a:?}"), "Secret(..)");
        let made = SecretRead::Made { secret: a, saved: Ok(()) };
        assert!(!format!("{made:?}").contains(&b.to_hex()[..8]));
    }
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core habits` and `cargo test -p cue-engines keys`
  (from `C:\Users\seymurglv\Desktop\cue-dev`). Expected: compile errors, `Secret`, `hmac_sha256`, `rekey`, `kept_secret`
  and `new_secret` are not defined and `hash` takes no secret.

- [ ] **Step 3: the dependencies.** In the workspace `Cargo.toml`, after `sha2 = "0.10"`:

```toml
# The habits' hash secret (cue-engines::keys): 32 random bytes from the system. Already in Cargo.lock through tauri.
getrandom = "0.3"
```

  In `app/crates/core/Cargo.toml`'s `[dependencies]`: `sha2.workspace = true` (the habits' keyed hash). In
  `app/crates/engines/Cargo.toml`'s `[dependencies]`, after `sha2.workspace = true`: `getrandom.workspace = true`.

- [ ] **Step 4: the secret and the keyed hash** (`habits.rs`). Imports: `use sha2::{Digest, Sha256};` after the `serde`
  line. Above `/// A kind of correction with its detail; counted by its [`Form::hash`].`:

```rust
/// The length of the hashes' secret, in bytes.
pub const SECRET_BYTES: usize = 32;

/// This installation's secret the forms' hashes are made with (docs/specs/2026-11-04-habits.md, "The hash's secret"):
/// 32 random bytes kept in Windows Credential Manager, never in settings.json, so that hashing common words does not
/// tell which ones `habits.seen` and `habits.declined` count. All zero is no secret, and nothing is counted with it.
/// `Debug` shows nothing of it; only the key store gets its hex digits (`to_hex`).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret([u8; SECRET_BYTES]);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_set() { "Secret(..)" } else { "Secret(none)" })
    }
}

impl Secret {
    pub fn from_bytes(bytes: [u8; SECRET_BYTES]) -> Self {
        Self(bytes)
    }

    /// Made: not all zero.
    pub fn is_set(&self) -> bool {
        self.0.iter().any(|b| *b != 0)
    }

    /// As the key store keeps it: 64 lower-case hex digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// From the key store's 64 hex digits; anything else, or all zero, is none.
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 2 * SECRET_BYTES || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut bytes = [0u8; SECRET_BYTES];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Self(bytes)).filter(Secret::is_set)
    }
}

/// HMAC-SHA-256 (RFC 2104) of `message` under `key`.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| k.map(|b| b ^ byte);
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(message).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}
```

  `Form::hash` (line 92) takes the secret; its `canon` stays as it is, and the last line hashes it under the secret:

```rust
    /// What the form is counted by: its kind and detail hashed under this installation's secret (HMAC-SHA-256, its
    /// first 8 bytes as 16 hex digits). Never its text, and without the secret no guess at its words.
    pub fn hash(&self, secret: &Secret) -> String {
        let canon = match self {
            // … the five arms as they are …
        };
        hmac_sha256(&secret.0, canon.as_bytes())[..8].iter().map(|b| format!("{b:02x}")).collect()
    }
```

  `correction` (line 170) keeps each form once by the form itself, not its hash (it has no secret):

```rust
    let mut forms = Vec::new();
    found.retain(|f| {
        let first = !forms.contains(&f.form);
        forms.push(f.form.clone());
        first
    });
```

  `Habit::hashes` (line 369):

```rust
    pub fn hashes(&self, secret: &Secret) -> Vec<String> {
        self.forms.iter().map(|f| f.hash(secret)).collect()
    }
```

  `Habits` (line 395): its doc comment ends "… Each list is written only when it is not empty. The secret the hashes are
  made with is never written: the runtime sets it from the key store."; after `declined`:

```rust
    #[serde(skip)]
    pub secret: Secret,
```

  `is_default` (line 479) no longer compares the whole struct (a set secret would write `"habits": {}`):

```rust
    /// Nothing counted, learned or declined (the secret aside).
    pub fn is_default(&self) -> bool {
        self.seen.is_empty() && self.list.is_empty() && self.declined.is_empty()
    }
```

  In `count` (line 494): the doc comment ends "… the least recently seen. Without the secret nothing is counted.", the
  first check becomes `if app.is_empty() || !self.secret.is_set() {`, and every `f.form.hash()` becomes
  `f.form.hash(&self.secret)`, every `h.hashes()` `h.hashes(&self.secret)` (the closures borrow `self.secret` beside
  `self.list`: disjoint fields). In `listed` and `adopt`: `let id = id_of(&draft.forms, &self.secret);`. In `forget`:
  `for hash in self.list.remove(i).hashes(&self.secret) {`. `forget_all` keeps the secret, and `rekey` follows it:

```rust
    /// Every habit goes, with the counts and the declined forms; the secret stays. False when there was nothing.
    pub fn forget_all(&mut self) -> bool {
        let changed = !self.is_default();
        self.seen.clear();
        self.list.clear();
        self.declined.clear();
        changed
    }

    /// A new secret, the one kept being missing or unreadable: the counts and the declined forms were hashed with the
    /// old one and go; the habits keep their forms, and their ids are made again. How many counts and declined forms
    /// went.
    pub fn rekey(&mut self, secret: Secret) -> (usize, usize) {
        let dropped = (self.seen.len(), self.declined.len());
        self.seen.clear();
        self.declined.clear();
        for h in &mut self.list {
            h.id = id_of(&h.forms, &secret);
        }
        self.secret = secret;
        dropped
    }
```

  `id_of` (line 639):

```rust
/// A habit's id: the hash of its forms' hashes (made with the secret).
pub fn id_of(forms: &[Form], secret: &Secret) -> String {
    let hashes: Vec<String> = forms.iter().map(|f| f.hash(secret)).collect();
    format!("h{:016x}", fnv1a(hashes.join(",").as_bytes()))
}
```

  The module comment's second paragraph: "A form is counted by its hash under this installation's secret, never by its
  text. …".

- [ ] **Step 5: the store** (`keys.rs`). Imports: `use cue_core::habits::Secret;` after `use crate::error::EngineError;`.
  After `set`:

```rust
/// The habits' hash secret's entry in the store, beside the connections' keys (docs/specs/2026-11-04-habits.md).
const HABIT_SECRET: &str = "habits-secret";

/// The habits' hash secret as `habit_secret` found it.
#[derive(Debug, PartialEq)]
pub enum SecretRead {
    /// The one the store keeps.
    Kept(Secret),
    /// None was kept, or it could not be read: a new one, and whether the store took it.
    Made { secret: Secret, saved: Result<(), EngineError> },
}

/// The secret the store gave back: 64 hex digits. Missing, unreadable or anything else is none. A keyring error goes
/// nowhere: `BadEncoding`'s `Debug` holds the stored bytes.
fn kept_secret(read: Result<String, keyring::Error>) -> Option<Secret> {
    read.ok().and_then(|hex| Secret::from_hex(hex.trim()))
}

/// 32 random bytes from the system.
fn new_secret() -> Result<Secret, EngineError> {
    let mut bytes = [0u8; cue_core::habits::SECRET_BYTES];
    getrandom::fill(&mut bytes).map_err(|e| EngineError::Service(format!("no random bytes for the habits' secret: {e}")))?;
    Ok(Secret::from_bytes(bytes))
}

/// The habits' hash secret kept in the store, without making one (the leak scan).
pub fn kept_habit_secret() -> Option<Secret> {
    kept_secret(keyring::Entry::new(SERVICE, HABIT_SECRET).and_then(|e| e.get_password()))
}

/// The habits' hash secret: the one kept, else a new one, saved. A save that fails leaves the new one for this session
/// (the next start makes another). `Err` only when the system gives no random bytes. No text holds the secret.
pub fn habit_secret() -> Result<SecretRead, EngineError> {
    if let Some(secret) = kept_habit_secret() {
        return Ok(SecretRead::Kept(secret));
    }
    let secret = new_secret()?;
    let saved = keyring::Entry::new(SERVICE, HABIT_SECRET)
        .and_then(|e| e.set_password(&secret.to_hex()))
        .map_err(|e| EngineError::Service(format!("could not save the habits' secret: {e}")));
    Ok(SecretRead::Made { secret, saved })
}
```

  The module comment gains a line: "and the habits' hash secret (`habit_secret`), under its own name; the same rule
  holds for it."

- [ ] **Step 6: the runtime sets it before the first event.** In `run` (line 472), after `forget_soniox_key();`:
  `self.key_habits();`. After `cleanup_key`:

```rust
    /// The habits' hash secret from the key store (docs/specs/2026-11-04-habits.md, "The hash's secret"), before the
    /// first correction is counted. A new one (none was kept, or it could not be read) leaves the counts and the
    /// declined forms meaningless: they go, the habits' ids are made again, and the file is written. Without random
    /// bytes nothing is counted this session. The log never gets the secret.
    fn key_habits(&mut self) {
        match keys::habit_secret() {
            Ok(keys::SecretRead::Kept(secret)) => self.settings.habits.secret = secret,
            Ok(keys::SecretRead::Made { secret, saved }) => {
                if let Err(e) = saved {
                    tracing::warn!("{e}");
                }
                let (seen, declined) = self.settings.habits.rekey(secret);
                let habits = self.settings.habits.list.len();
                tracing::info!(seen, declined, habits, "habits: a new hash secret, counts and declined forms dropped");
                if seen + declined + habits > 0 {
                    self.save_settings();
                }
            }
            Err(e) => tracing::warn!("{e}: no habit is counted"),
        }
    }
```

- [ ] **Step 7: the leak scan looks for it** like a saved key (`leak.rs`, `gather`, after the keys' loop, line 192):

```rust
        // The habits' hash secret, as the store keeps it (never made here).
        if let Some(secret) = keys::kept_habit_secret() {
            private.add(Kind::SavedKey("habits' secret"), &secret.to_hex(), 1);
        }
```

  The module comment's list says "the keys saved in Credential Manager (the habits' hash secret among them)".

- [ ] **Step 8: README and ARCHITECTURE.**
  - README, Privacy (line 89), after "…are the only text of a correction it writes to disk.": "What it counts on the
    way is kept by a hash made with a random secret of this computer, which Windows Credential Manager keeps beside the
    keys (never in `settings.json`), so a copy of the file does not tell which words were counted."
  - README, `habits.seen` (line 534): "the form by its hash (made with this computer's secret) and no text".
  - README, privacy in detail, the **Habits** bullet (line 750): after "…a number and a day, no text.": "The hash is
    made with a random secret of this installation (32 bytes, an HMAC), kept in Windows Credential Manager under
    `habits-secret` beside the keys, never in `settings.json` or the log. When it is missing or cannot be read, CUE
    makes a new one at start and drops the counts and the declined corrections (their hashes no longer match); the
    habits keep working."
  - ARCHITECTURE, Habits, step 2: the sentence "A form is counted by its hash, FNV-1a written out (`Form::hash`),
    because std's hasher is not stable between versions." becomes "A form is counted by its hash under this
    installation's secret (`Form::hash(&Secret)`: HMAC-SHA-256 of its kind and detail, written out over `sha2`, the
    first 8 bytes; a habit's id is FNV-1a over its forms' hashes). The secret (`habits::Secret`, 32 random bytes) is
    kept in Credential Manager (`keys::habit_secret`, the entry `habits-secret`); `Runtime::run` sets it before the
    first event (`key_habits`), and a new one goes through `Habits::rekey`: `seen` and `declined` go, the ids are made
    again. It never reaches settings.json (`#[serde(skip)]`) or a log (`Secret(..)`), and without one nothing is
    counted."

- [ ] **Step 9: run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings` (from
  `C:\Users\seymurglv\Desktop\cue-dev`): green. `Cargo.lock` gains `sha2` under `cue-core` and `getrandom 0.3.4` under
  `cue-engines`, nothing new downloaded.

---

### Task 12: stricter widening (a habit's whole set, three times in another app)

Decided 06.11.2026 (spec, "Wider", changed).

**Files:**
- Modify: `app/crates/core/src/habits.rs` (`Counted`, `Seen`, `Habits::count`, `Habits::forget`, tests)
- Modify: `README.md`, `docs/ARCHITECTURE.md`

**Interfaces:**
- Consumes: Task 11 (`hash(&self.secret)`, `habits()` in the tests).
- Produces: no new names. `Habits::count` widens a habit of another app only when all its forms came together in one
  correction `LEARN_AT` times there; each such correction bumps one `seen` entry `{ app, form: <the habit's id>, n, last }`.
  A form of a habit of this app or of everywhere grows that habit and is in no set; the other forms count for this app
  as any form does, and can make its own habit. `forget` drops the habit's set counts too.

- [ ] **Step 1: the tests.** Rename `a_habit_widens_when_its_form_reaches_three_in_another_app` to
  `a_habit_of_one_form_widens_when_it_comes_three_times_in_another_app` with the doc comment "/// A habit of one form:
  the form alone is its whole set." (its body stays: the dash alone is the habit's set).
  `a_habit_of_two_forms_grows_by_one_per_correction_and_widening_adds_only_its_count` stays as it is (Word gets both forms
  together each time). Replace `widening_removes_the_counts_of_all_its_forms_in_every_app` with:

```rust
    #[test]
    fn widening_removes_the_counts_of_its_set_and_its_forms_in_every_app() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count("Notepad", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        h.count("Notepad", &correction("Скинь файл.", "Скинь файл"), "2026-11-04", "en");
        let seen: Vec<(&str, &str, u32)> = h.seen.iter().map(|s| (s.app.as_str(), s.form.as_str(), s.n)).collect();
        let stop = STOP.hash(&h.secret);
        assert_eq!(seen, [("Notepad", stop.as_str(), 1), ("Notepad", id.as_str(), 1)], "the set under the habit's id");
        let c = correction("Скинь файл.", "скинь файл");
        h.count("Word", &c, "2026-11-04", "en");
        h.count("Word", &c, "2026-11-04", "en");
        h.count("Word", &c, "2026-11-04", "en");
        assert!(h.seen.is_empty(), "{:?}", h.seen);
    }
```

  And add, before `a_replacement_outside_the_stored_example_is_not_drafted_but_still_counted`:

```rust
    /// Widening asks for the habit's whole set in one dictation: its forms seen apart in the other app count for that
    /// app, as any form does.
    #[test]
    fn all_its_forms_together_three_times_in_another_app_widen_a_habit() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        let both = correction("Скинь файл.", "скинь файл");
        let stop = correction("Скинь файл.", "Скинь файл");
        h.count("Word", &both, "2026-11-04", "en");
        h.count("Word", &stop, "2026-11-04", "en");
        h.count("Word", &both, "2026-11-04", "en");
        let seen: Vec<(&str, u32)> = h.seen.iter().map(|s| (s.form.as_str(), s.n)).collect();
        assert_eq!(seen, [(id.as_str(), 2), (STOP.hash(&h.secret).as_str(), 1)], "the set twice, the full stop alone once");
        assert_eq!(h.list[0].app.as_deref(), Some(TG));
        let counted = h.count("Word", &both, "2026-11-04", "en");
        assert_eq!((counted.widened, counted.new), (vec![id], None));
        let habit = &h.list[0];
        assert_eq!((habit.app.as_deref(), habit.n, habit.rule.as_str()), (None, 6, "Lower case at the start and no full stop at the end"));
        assert!(h.seen.is_empty());
    }

    /// One form of a habit, three times alone in another app: no widening, but that app's own habit.
    #[test]
    fn one_form_of_a_habit_alone_in_another_app_makes_that_apps_own_habit() {
        let mut h = habits();
        let first = learned(&mut h, "Скинь файл.", "скинь файл", "en");
        let stop = correction("Скинь файл.", "Скинь файл");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &stop, "2026-11-04", "en");
        }
        assert!(counted.widened.is_empty() && !counted.grown);
        let draft = counted.new.expect("Word's own");
        assert_eq!((draft.forms.as_slice(), draft.app.as_str()), ([STOP].as_slice(), "Word"));
        let word = h.adopt(draft, None, "en");
        assert_eq!(h.list, [word.clone(), first.clone()], "the first stays in its app");
        assert_eq!(word.rule, "In Word: no full stop at the end");
        assert_eq!(h.for_app("Word").map(|x| x.id.as_str()).collect::<Vec<_>>(), [word.id.as_str()]);
        assert_eq!(h.for_app(TG).map(|x| x.id.as_str()).collect::<Vec<_>>(), [first.id.as_str()]);
        // In Word now, the full stop counts on Word's habit and the first letter alone; the set is not whole.
        let counted = h.count("Word", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        assert!(counted.grown && counted.widened.is_empty());
        assert_eq!(h.list[0].n, 4);
        assert_eq!(h.seen.iter().map(|s| s.form.clone()).collect::<Vec<_>>(), [Form::FirstLetter { lower: true }.hash(&h.secret)]);
    }

    #[test]
    fn a_forgotten_habit_takes_its_sets_counts_along() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count("Word", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        assert_eq!(h.seen.len(), 1);
        assert!(h.forget(&id));
        assert!(h.seen.is_empty());
    }
```

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core habits`. Expected: the four new or rewritten
  tests fail (the old `count` widens on the full stop alone and keeps per-form counts in Word); the others pass.

- [ ] **Step 3: `count` counts a habit's set.** Replace `count`, from its doc comment down to the line before
  `// Only replacements whose words stand in the stored example are drafted; …` (that part and the draft after it stay
  as they are), with:

```rust
    /// One correction of a dictation in `app` (the history's name for it, `today` its local date): each form counts
    /// once. A declined form is not counted. A form of a habit of this app or of everywhere adds to that habit's count.
    /// A habit of another app whose forms all came here together counts as one entry of this app, under its id; at
    /// `LEARN_AT` it widens (its rule loses its app, its counts add up). The other forms count for this app, and those
    /// that reach `LEARN_AT` together become one draft. The counts that went into a habit go; counts older than
    /// `SEEN_DAYS` go, and past `MAX_SEEN` the least recently seen. Without the secret nothing is counted.
    pub fn count(&mut self, app: &str, c: &Correction, today: &str, lang: &str) -> Counted {
        let mut out = Counted::default();
        if app.is_empty() || !self.secret.is_set() {
            return out;
        }
        self.prune(today);
        let live: Vec<(&Found, String)> =
            c.found.iter().map(|f| (f, f.form.hash(&self.secret))).filter(|(_, hash)| !self.declined.contains(hash)).collect();
        // The forms counted on a habit already, or in a habit's set.
        let mut handled: Vec<String> = Vec::new();
        let mut grown: Vec<String> = Vec::new();
        for (_, hash) in &live {
            let here = |h: &&Habit| h.app.as_deref().is_none_or(|a| a == app) && h.hashes(&self.secret).contains(hash);
            if let Some(h) = self.list.iter().find(here) {
                if !grown.contains(&h.id) {
                    grown.push(h.id.clone());
                }
                handled.push(hash.clone());
            }
        }
        let free: Vec<&String> = live.iter().map(|(_, hash)| hash).filter(|hash| !handled.contains(hash)).collect();
        let sets: Vec<(String, Vec<String>)> = self
            .list
            .iter()
            .filter(|h| h.app.as_deref().is_some_and(|a| a != app))
            .map(|h| (h.id.clone(), h.hashes(&self.secret)))
            .filter(|(_, hashes)| hashes.iter().all(|hash| free.contains(&hash)))
            .collect();
        let mut widened: Vec<(String, u32)> = Vec::new();
        for (id, hashes) in sets {
            handled.extend(hashes);
            let n = self.bump(app, &id, today);
            if n >= LEARN_AT {
                widened.push((id, n));
            }
        }
        for (id, n) in &widened {
            if let Some(h) = self.list.iter_mut().find(|h| &h.id == id) {
                let was = h.app.take().unwrap_or_default();
                h.rule = unscoped(&h.rule, &was, &h.forms, lang);
                h.n = h.n.saturating_add(*n);
                // Its set's counts and its forms' counts, in every app.
                let mut gone = h.hashes(&self.secret);
                gone.push(id.clone());
                self.seen.retain(|s| !gone.contains(&s.form));
            }
        }
        // A habit's count grows once per correction.
        for h in &mut self.list {
            if grown.contains(&h.id) {
                h.n = h.n.saturating_add(1);
            }
        }
        out.grown = !grown.is_empty();
        out.widened = widened.into_iter().map(|(id, _)| id).collect();
        let mut reached: Vec<&Found> = Vec::new();
        for (f, hash) in &live {
            if !handled.contains(hash) && self.bump(app, hash, today) >= LEARN_AT {
                reached.push(f);
            }
        }
```

  `Counted.widened`'s comment: "Habits of another app whose forms came together here `LEARN_AT` times: they hold
  everywhere now." `Seen`'s comment: "How many times a form was corrected in an app, or a habit of another app was
  corrected there whole, and the day it last was: no text (`form` is the form's hash, or the habit's id)." `forget`'s
  comment ends "…never learned again; its set's counts go too. False when there is no such habit.", and its first line
  after the `let Some(i)` is `self.seen.retain(|s| s.form != id);`.

- [ ] **Step 4: run** `cargo test -p cue-core` and `cargo clippy -p cue-core --all-targets -- -D warnings`: green.

- [ ] **Step 5: README and ARCHITECTURE.**
  - README, the **Habits** bullet (line 278): "When the same correction reaches three in another app, the habit holds in
    every app: its row says so, and no card opens." becomes "When all of a habit's corrections come together in one
    dictation three times in another app, the habit holds in every app: its row says so, and no card opens. One of
    them alone there counts for that app, and can become that app's own habit."
  - README, `habits.seen` (line 534): add "; a habit's whole set corrected together in another app is counted as one
    entry, under the habit's id."
  - ARCHITECTURE, Habits, step 3: "A form that reaches `LEARN_AT` (3) and is a habit of another app widens it: its app
    is cleared, the rule loses its "In <app>" start (`habits::unscoped`), the counts add up, and `send_settings` redraws
    the section without a card." becomes "A habit of another app whose forms all came together in the correction counts
    once for this app, under its id (a form already counted on a habit of this app or of every app is in no set); when
    that set reaches `LEARN_AT` (3) the habit widens: its app is cleared, the rule loses its "In <app>" start
    (`habits::unscoped`), the counts add up, its set's and its forms' counts go in every app, and `send_settings`
    redraws the section without a card. A habit's form alone in another app counts for that app as any form does."

- [ ] **Step 6: run** `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`: green.

---

### Task 13: opposite habits ("Habit changed")

Decided 06.11.2026 (spec, "Opposite forms" and "Changed").

**Files:**
- Modify: `app/crates/core/src/habits.rs`, `app/crates/core/src/ui.rs` (`TrayKind`, line 61; its test, line 155)
- Modify: `app/src-tauri/src/runtime.rs`
- Modify: `app/ui/src/strings.ts`
- Test: `app/ui/test/controller.test.ts`
- Modify: `README.md`, `docs/ARCHITECTURE.md`

**Interfaces:**
- Consumes: Tasks 11 and 12.
- Produces in `habits`: `Form::opposite(&self) -> Option<Form>` (end mark removed ↔ added, lower case ↔ capital,
  `A → B` ↔ `B → A`; digits and dash none); `fn opposes(a: &[Form], b: &[Form]) -> bool`;
  `#[derive(Debug, PartialEq)] pub struct Learned { pub habit: Habit, pub replaced: Vec<Habit>, pub merged: bool }`;
  `Habits::learn(&mut self, Draft, Option<RuleText>, lang) -> Learned` (`adopt` becomes `learn(..).habit`, its callers
  unchanged); `fn Habits::opposed(&self, forms, id) -> bool`; `Habits::revert(&mut self, id: &str, replaced: Vec<Habit>,
  decline: bool) -> bool`. `count` lets no habit with an opposite habit in some app widen.
- Produces in `ui`: `TrayKind::HabitChanged` (JSON `"habitChanged"`), drawn like `newHabit` with the rule just before
  it on `cue://habit-rule`; its "Undo" is `UiAction::TrayAction` → `Habits::revert(id, replaced, true)`.
- Produces in the runtime: `struct HabitCard { id: String, replaced: Vec<habits::Habit> }` with `kind()`;
  `habit_cards: VecDeque<(u64, HabitCard)>`, `habit_card: Option<HabitCard>`; `undo_habit(&mut self, HabitCard)`.
- Produces in `strings.ts`: `TrayKind` gains `'habitChanged'`; `TRAYS.habitChanged = { icon: 'habit', tone: 'ok',
  act: 'undo', ttl: 7000 }`; `en.tray.habitChanged` / `ru.tray.habitChanged` (same keys as `newHabit`).

- [ ] **Step 1: the failing tests.** In `habits.rs`'s tests, before
  `a_replacement_outside_the_stored_example_is_not_drafted_but_still_counted`:

```rust
    const ADDED: Form = Form::EndMark { mark: '.', removed: false };

    #[test]
    fn each_form_has_its_opposite_but_digits_and_a_dash() {
        assert_eq!((STOP.opposite(), ADDED.opposite()), (Some(ADDED), Some(STOP)));
        assert_eq!(Form::FirstLetter { lower: true }.opposite(), Some(Form::FirstLetter { lower: false }));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert_eq!(hello.opposite(), Some(Form::Replacement { from: "добрый день".into(), to: "здравствуйте".into() }));
        assert_eq!((Form::Digits.opposite(), Form::Dash.opposite()), (None, None));
    }

    /// The user goes back on a habit: the opposite counts like any form, and at three the habit changes. The old one
    /// goes without being declined; "Undo" brings it back as it was and declines the new forms.
    #[test]
    fn an_opposite_three_times_changes_the_habit_and_undo_brings_it_back() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        assert_eq!(kinds("Скинь файл", "Скинь файл."), [ADDED]);
        let back = correction("Скинь файл", "Скинь файл.");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count(TG, &back, "2026-11-04", "en");
            assert!(!counted.grown, "the habit's own count does not grow");
        }
        let changed = h.learn(counted.new.expect("the opposite at three"), None, "en");
        assert_eq!((changed.replaced.as_slice(), changed.merged), ([old.clone()].as_slice(), false));
        assert_eq!((changed.habit.rule.as_str(), changed.habit.app.as_deref()), ("In Telegram Desktop: a full stop at the end", Some(TG)));
        assert_eq!(h.list, std::slice::from_ref(&changed.habit), "the two never live together");
        assert!(h.declined.is_empty(), "the old habit may be learned again");
        assert!(h.revert(&changed.habit.id, changed.replaced, true));
        assert_eq!(h.list, [old]);
        assert_eq!(h.declined, [ADDED.hash(&h.secret)]);
        assert_eq!(h.count(TG, &back, "2026-11-04", "en"), Counted::default(), "the new form is declined");
        assert!(!h.revert("nope", Vec::new(), true));
    }

    /// An opposite in another app is that app's own habit: the first one is not changed.
    #[test]
    fn an_opposite_in_another_app_leaves_the_habit_alone() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        let back = correction("Скинь файл", "Скинь файл.");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &back, "2026-11-04", "en");
        }
        let word = h.learn(counted.new.expect("Word's own"), None, "en");
        assert!(word.replaced.is_empty());
        assert_eq!(h.list, [word.habit, old]);
    }

    /// A habit of everywhere is changed by an opposite reaching three in one app, and the change holds everywhere: the
    /// app habits it goes back on go too. Without `decline`, reverting leaves the new forms free.
    #[test]
    fn a_change_of_a_habit_of_everywhere_holds_everywhere() {
        let mut h = habits();
        let draft = |forms: Vec<Form>, app: &str| Draft { forms, app: app.into(), before: "a".into(), after: "b".into(), n: 3 };
        h.adopt(draft(vec![Form::FirstLetter { lower: true }], TG), None, "en");
        h.list[0].app = None; // as if it had widened
        h.adopt(draft(vec![STOP], "Notepad"), None, "en");
        let dash = h.adopt(draft(vec![Form::Dash], "Word"), None, "en");
        let before = h.list.clone();
        let back = correction("скинь файл", "Скинь файл.");
        assert_eq!(back.found.iter().map(|f| f.form.clone()).collect::<Vec<_>>(), [Form::FirstLetter { lower: false }, ADDED]);
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &back, "2026-11-04", "en");
        }
        let changed = h.learn(counted.new.expect("in Word"), None, "en");
        assert_eq!(changed.replaced, before[1..], "the full stop of Notepad and the lower case of everywhere");
        assert_eq!((changed.habit.app.as_deref(), changed.habit.rule.as_str()), (None, "A capital at the start and a full stop at the end"));
        assert_eq!(h.list, [changed.habit.clone(), dash]);
        assert!(h.revert(&changed.habit.id, changed.replaced, false));
        assert!(h.declined.is_empty());
        let mut back_again: Vec<String> = h.list.iter().map(|x| x.id.clone()).collect();
        let mut was: Vec<String> = before.iter().map(|x| x.id.clone()).collect();
        back_again.sort();
        was.sort();
        assert_eq!(back_again, was, "the three as they were");
    }

    /// A habit with an opposite habit in another app never holds everywhere, where the two would live together:
    /// its forms count for a third app on their own, and a second draft of them only adds to its count.
    #[test]
    fn a_habit_with_an_opposite_in_another_app_never_widens() {
        let mut h = habits();
        let tg = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        let word = Draft { forms: vec![ADDED], app: "Word".into(), before: "a".into(), after: "a.".into(), n: 3 };
        let word = h.adopt(word, None, "en");
        let stop = correction("Скинь файл.", "Скинь файл");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Notepad", &stop, "2026-11-04", "en");
        }
        assert!(counted.widened.is_empty());
        let merged = h.learn(counted.new.expect("its form counted for Notepad"), None, "en");
        assert!(merged.merged && merged.replaced.is_empty());
        assert_eq!((merged.habit.id.as_str(), merged.habit.app.as_deref(), merged.habit.n), (tg.id.as_str(), Some(TG), 6));
        assert_eq!(h.list.len(), 2);
        assert_eq!(h.list[0], word);
    }
```

  In `ui.rs`'s `every_variant_has_its_json_name`, add `(TrayKind::HabitChanged, "habitChanged")` to `trays`. In
  `runtime.rs`'s tests, after `the_notice_found_first_opens_first`:

```rust
    /// A habit's card says "New habit", or "Habit changed" when the habit took the place of others.
    #[test]
    fn a_card_that_replaced_a_habit_says_it_changed() {
        let fresh = HabitCard { id: "h1".into(), replaced: Vec::new() };
        assert_eq!(fresh.kind(), TrayKind::NewHabit);
        let d = habits::Draft { forms: vec![habits::Form::Dash], app: "Word".into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        let old = habits::Habits::default().adopt(d, None, "en");
        assert_eq!(HabitCard { id: "h2".into(), replaced: vec![old] }.kind(), TrayKind::HabitChanged);
    }
```

  In `controller.test.ts`, add `'habitChanged'` to `KINDS` (line 24), and after the `'New habit card'` block (it ends at
  line 1291):

```ts
describe('Habit changed card', () => {
  const box = () => document.querySelector('.tray') as HTMLElement;
  const isOpen = () => box().classList.contains('open');
  const act = () => box().querySelector('[data-act]') as HTMLButtonElement;
  const RULE = 'In Telegram — a full stop at the end';

  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it('says the new rule under "Habit changed", with the habit icon in green', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    expect(box().querySelector('.nhd b')!.textContent).toBe('Habit changed');
    expect(box().querySelector('.nbd')!.textContent).toBe(`${RULE}. CUE writes it this way from now on.`);
    const nic = box().querySelector('.nic')!;
    expect(nic.classList.contains('tone-ok')).toBe(true);
    const icon = document.createElement('span');
    icon.innerHTML = CUE().IC.habit;
    expect(nic.innerHTML).toBe(icon.innerHTML);
    expect(act().querySelector('.lbl')!.getAttribute('data-t')).toBe('Undo');
  });

  it('closes by itself after 7 s', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    vi.advanceTimersByTime(7000 - 1);
    expect(isOpen()).toBe(true);
    vi.advanceTimersByTime(1);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayClose']);
  });

  it('"Undo" turns into "Restored", tells the core, and the card closes a moment later', () => {
    ctl.habitRule(RULE);
    ctl.tray('habitChanged');
    act().click();
    expect(sent).toEqual(['trayAction']);
    expect([...box().querySelectorAll('.lbl span')].map((s) => s.textContent).join('')).toBe('Restored');
    expect(act().disabled).toBe(true);
    vi.advanceTimersByTime(1100);
    expect(isOpen()).toBe(false);
    expect(sent).toEqual(['trayAction']);
  });
});
```

  And in the "Russian interface" block, after the habit card's test:

```ts
  it('the changed habit card in Russian: «Отменить» becomes «Возвращено»', () => {
    ctl.habitRule('В Telegram — с точкой в конце');
    ctl.tray('habitChanged');
    expect(document.querySelector('.tray .nhd b')!.textContent).toBe('Привычка изменилась');
    expect(document.querySelector('.tray .nbd')!.textContent).toBe('В Telegram — с точкой в конце. Дальше CUE пишет так сам.');
    (document.querySelector('.tray [data-act]') as HTMLButtonElement).click();
    expect([...document.querySelectorAll('.tray .lbl span')].map((s) => s.textContent).join('')).toBe('Возвращено');
  });
```

  Run `cargo test -p cue-core`, `cargo test -p cue` and `npx vitest run test/controller.test.ts` (from `app`) and see
  them fail: `opposite`, `learn`, `revert`, `HabitChanged`, `HabitCard` and `'habitChanged'` are not defined.

- [ ] **Step 2: opposites in the core** (`habits.rs`). In `impl Form`, before `shows_in`:

```rust
    /// The form that goes back on this one (docs/specs/2026-11-04-habits.md, "Opposite forms"): the end mark added for
    /// one removed and back, the other case, the replacement the other way. Digits and a dash have none.
    pub fn opposite(&self) -> Option<Form> {
        match self {
            Self::EndMark { mark, removed } => Some(Self::EndMark { mark: *mark, removed: !removed }),
            Self::FirstLetter { lower } => Some(Self::FirstLetter { lower: !lower }),
            Self::Replacement { from, to } => Some(Self::Replacement { from: to.clone(), to: from.clone() }),
            Self::Digits | Self::Dash => None,
        }
    }
```

  Above `/// What counting one correction changed.`:

```rust
/// Forms that go back on each other: one of `a` has its opposite in `b`.
fn opposes(a: &[Form], b: &[Form]) -> bool {
    a.iter().any(|f| f.opposite().is_some_and(|o| b.contains(&o)))
}

/// What learning a draft did ([`Habits::learn`]).
#[derive(Debug, PartialEq)]
pub struct Learned {
    /// The habit as it is now.
    pub habit: Habit,
    /// The habits it changed, as they were: they held an opposite of its forms and are gone, not declined.
    pub replaced: Vec<Habit>,
    /// A habit of its forms was listed already ([`Habits::listed`]): the draft added to it.
    pub merged: bool,
}
```

  In `count`'s `sets` (Task 12), the first filter becomes
  `.filter(|h| h.app.as_deref().is_some_and(|a| a != app) && !self.opposed(&h.forms, &h.id))`. Replace `adopt` (its doc
  comment and body) with:

```rust
    /// The draft's habit, as [`Habits::learn`] makes it.
    pub fn adopt(&mut self, draft: Draft, rule: Option<RuleText>, lang: &str) -> Habit {
        self.learn(draft, rule, lang).habit
    }

    /// The draft's habit goes first, with the rule the cleanup wrote or else the template's; past `MAX_HABITS` the
    /// oldest go. Its id is the hash of its forms. The habits holding an opposite of its forms in its app or everywhere
    /// are changed by it (docs/specs/2026-11-04-habits.md, "Changed"): they go, not declined; when one of them held
    /// everywhere, the new habit does too, and those of every app go as well. When a habit of its forms is listed
    /// already ([`Habits::listed`]), the draft adds to it instead, as a widening would: its count adds up, and a draft
    /// of another app makes it hold everywhere (its rule loses its app, the draft's rule is not used) unless an opposite
    /// habit lives in some app ([`Habits::opposed`]).
    pub fn learn(&mut self, draft: Draft, rule: Option<RuleText>, lang: &str) -> Learned {
        let opposite = |h: &Habit| opposes(&h.forms, &draft.forms);
        let everywhere = self.list.iter().any(|h| h.app.is_none() && opposite(h));
        let (replaced, kept): (Vec<Habit>, Vec<Habit>) = std::mem::take(&mut self.list)
            .into_iter()
            .partition(|h| opposite(h) && (everywhere || h.app.as_deref() == Some(draft.app.as_str())));
        self.list = kept;
        let id = id_of(&draft.forms, &self.secret);
        let opposed = self.opposed(&draft.forms, &id);
        if let Some(h) = self.list.iter_mut().find(|h| h.id == id) {
            if !opposed
                && (everywhere || h.app.as_deref().is_some_and(|a| a != draft.app))
                && let Some(was) = h.app.take()
            {
                h.rule = unscoped(&h.rule, &was, &h.forms, lang);
            }
            h.n = h.n.saturating_add(draft.n);
            return Learned { habit: h.clone(), replaced, merged: true };
        }
        let RuleText { rule, short } = rule.unwrap_or_else(|| template(&draft.forms, Some(&draft.app), lang));
        let (rule, app) =
            if everywhere { (unscoped(&rule, &draft.app, &draft.forms, lang), None) } else { (rule, Some(draft.app)) };
        let habit = Habit { id, forms: draft.forms, rule, short, before: draft.before, after: draft.after, app, n: draft.n, on: true };
        self.list.insert(0, habit.clone());
        self.list.truncate(MAX_HABITS);
        Learned { habit, replaced, merged: false }
    }

    /// A habit other than `id` holds an opposite of one of `forms`: a habit of them never holds everywhere, where the
    /// two would live together.
    fn opposed(&self, forms: &[Form], id: &str) -> bool {
        self.list.iter().any(|h| h.id != id && opposes(&h.forms, forms))
    }

    /// A change undone: the habit `id` goes and the habits it replaced come back as they were, first. With `decline`
    /// (the card's "Undo") its forms are never learned again; without (a change dropped before its card, "Learn from
    /// me" off) they may be. False when there is no such habit.
    pub fn revert(&mut self, id: &str, replaced: Vec<Habit>, decline: bool) -> bool {
        let gone = if decline {
            self.forget(id)
        } else {
            let before = self.list.len();
            self.list.retain(|h| h.id != id);
            self.list.len() != before
        };
        if !gone {
            return false;
        }
        for h in replaced.into_iter().rev() {
            self.list.retain(|x| x.id != h.id);
            self.list.insert(0, h);
        }
        self.list.truncate(MAX_HABITS);
        true
    }
```

  Run `cargo test -p cue-core` and `cargo clippy -p cue-core --all-targets -- -D warnings`: green.

- [ ] **Step 3: the kind.** In `ui.rs` after `NewHabit`:

```rust
    /// A habit took the place of its opposite (docs/specs/2026-11-04-habits.md, "Changed"): the new rule comes to the
    /// Drop just before the card, and the card's button brings the old habit back.
    HabitChanged,
```

- [ ] **Step 4: the runtime.** Next to `Notice` (line 2629):

```rust
/// A habit's card: the habit learned, and the habits it changed (docs/specs/2026-11-04-habits.md, "Changed"), none for a
/// new one. Its "Undo" brings those back as they were.
#[derive(Clone, Debug, PartialEq)]
struct HabitCard {
    id: String,
    replaced: Vec<habits::Habit>,
}

impl HabitCard {
    fn kind(&self) -> TrayKind {
        if self.replaced.is_empty() { TrayKind::NewHabit } else { TrayKind::HabitChanged }
    }
}
```

  `struct Runtime` (line 318): `habit_cards: VecDeque<(u64, HabitCard)>,` and `habit_card: Option<HabitCard>,` (the
  comment: "…in the order found, and the one open."). `on_habit_rule` (line 936), from `let from_cleanup` to its end:

```rust
        let from_cleanup = rule.is_some();
        // Another app's draft of the same forms came first (`merged`): this one adds to that habit, which holds
        // everywhere now, and its row changes with no second card. Habits that held an opposite of its forms went
        // (`replaced`): its card says "Habit changed".
        let habits::Learned { habit, replaced, merged } = self.settings.habits.learn(draft, rule, self.lang.code());
        self.save_settings();
        let kinds: Vec<&str> = habit.forms.iter().map(habits::Form::kind).collect();
        tracing::info!(?kinds, n = habit.n, from_cleanup, merged, replaced = replaced.len(), "habit learned");
        self.send_settings();
        if !merged || !replaced.is_empty() {
            self.habit_cards.push_back((n, HabitCard { id: habit.id, replaced }));
            self.show_notice();
        }
    }
```

  In `show_notice` (line 989): `self.habit_cards.retain(|(_, card)| list.iter().any(|h| h.id == card.id));`.
  `show_habit_card` (line 1004) and `send_habit_rule` (line 1016):

```rust
    /// The next habit's card: its rule to the Drop, then the card ("New habit" or "Habit changed"). One forgotten from
    /// the main window meanwhile has none.
    fn show_habit_card(&mut self) {
        while let Some((_, card)) = self.habit_cards.pop_front() {
            let Some(rule) = self.settings.habits.list.iter().find(|h| h.id == card.id).map(|h| h.rule.clone()) else { continue };
            self.tray = Some(card.kind());
            self.habit_card = Some(card);
            self.ui.habit_rule(&rule);
            self.ui.tray(self.tray);
            return;
        }
    }

    /// The rule of the habit whose card is open, before the card goes to a Drop loaded again.
    fn send_habit_rule(&self, kind: Option<TrayKind>) {
        if matches!(kind, Some(TrayKind::NewHabit | TrayKind::HabitChanged))
            && let Some(h) = self.habit_card.as_ref().and_then(|c| self.settings.habits.list.iter().find(|h| h.id == c.id))
        {
            self.ui.habit_rule(&h.rule);
        }
    }
```

  In `change_setting`, the "Learn from me" off block (line 819): a change waiting for its card is taken back, its
  replaced habits return (nothing is learned while off); the comment above the block adds "a change waiting for its
  card is taken back":

```rust
            let mut dropped = 0;
            for (_, card) in self.habit_cards.drain(..) {
                if self.settings.habits.revert(&card.id, card.replaced, false) {
                    dropped += 1;
                }
            }
            if dropped > 0 {
                tracing::info!(dropped, "habits not yet announced dropped");
                self.save_settings();
            }
```

  and the check after it (line 829): `if let Some(card) = &self.habit_card && !self.settings.habits.list.iter().any(|h| h.id == card.id)`.
  In `on_tray_action` (line 2063), the habit's arm and the hold:

```rust
            // "Undo" on a habit's card: the habit goes, and its forms are never learned again; a changed habit's card
            // brings back the habits it replaced, as they were.
            Some(TrayKind::NewHabit | TrayKind::HabitChanged) => {
                if let Some(card) = self.habit_card.take() {
                    self.undo_habit(card);
                }
            }
```

  `if matches!(kind, Some(TrayKind::NoField | TrayKind::NewHabit | TrayKind::HabitChanged)) {`, and after
  `on_tray_action`:

```rust
    /// A habit's card undone: a new habit is forgotten (`HabitForget`: its forms declined); a changed one goes the same
    /// way and the habits it replaced come back (`Habits::revert`). The main window redraws the section.
    fn undo_habit(&mut self, card: HabitCard) {
        if card.replaced.is_empty() {
            self.learn_word(SettingChange::HabitForget(card.id));
            tracing::info!("habit undone");
        } else if self.settings.habits.revert(&card.id, card.replaced, true) {
            self.save_settings();
            self.send_settings();
            tracing::info!("habit change undone");
        }
    }
```

  The module comment's habits bullet adds "; a correction that goes back on a habit three times changes it, and its
  card says so".

- [ ] **Step 5: `strings.ts`.** `TrayKind` (line 4) gains `| 'habitChanged'`; `TRAYS` gains
  `habitChanged: { icon: 'habit', tone: 'ok', act: 'undo', ttl: 7000 },`; `en.tray`, after `newHabit`:

```ts
    habitChanged: {
      title: 'Habit changed',
      body: '{RULE}. CUE writes it this way from now on.',
      act: 'Undo', done: 'Restored',
      bare: 'CUE writes it this way from now on.',
    },
```

  and `ru.tray`:

```ts
    habitChanged: {
      title: 'Привычка изменилась',
      body: '{RULE}. Дальше CUE пишет так сам.',
      act: 'Отменить', done: 'Возвращено',
      bare: 'Дальше CUE пишет так сам.',
    },
```

  `trays.ts` and `controller.ts` need nothing: the card takes `{RULE}` and the done label by its kind, as `newHabit`
  does, and it grows out of the Drop, pauses at the mouse and morphs "Undo" → "Restored" by shared letters like the
  other cards (reduced motion as they do).

- [ ] **Step 6: README and ARCHITECTURE.**
  - README, the **Habits** bullet, after "…can become that app's own habit." (Task 12): "When you go back on a habit in
    its app three times (the full stop it takes off added again, the capital back, the old words back), the habit
    changes: the old one goes, the new one is learned, and a "Habit changed" card says the new rule; its **Undo** brings
    the old habit back as it was and never learns the new one. A habit of every app changes the same way, from three
    times in any one app, and the change holds everywhere. Two opposite habits never live together, so a habit with an
    opposite in some app does not spread to every app."
  - ARCHITECTURE, Habits, step 5, after "…with no second card (`merged` in the log line).": "`adopt` is
    `Habits::learn`, which also takes out the habits holding an opposite of the draft's forms (`Form::opposite`: an end
    mark removed and added, lower case and a capital, a replacement and its reverse) in its app or in every app, not
    declined; when one of them held everywhere, the new habit does too and the opposites of every app go with it. They
    come back in `Learned.replaced`, kept on the card (`HabitCard`). A habit with an opposite habit in some app never
    widens (`Habits::opposed`), by its set or by a second draft." Step 6: "…then the plain tray kind `newHabit` on
    `cue://tray`" becomes "…then the plain tray kind `newHabit`, or `habitChanged` for a card with replaced habits, on
    `cue://tray`", and after "(the habit goes and its hashes join `declined`)": "; on `habitChanged`, `undo_habit` calls
    `Habits::revert(id, replaced, true)`, which also brings the replaced habits back (done label "Restored")". "Learn
    from me" turned off reverts a change waiting for its card without declining.

- [ ] **Step 7: run** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`; from `app`:
  `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`: green.

- [ ] **Step 8: live, with the owner** (as Task 10, Step 4):
  1. The first start after Tasks 11–13: the log has "habits: a new hash secret" with the numbers; `habits.seen` and
     `habits.declined` are gone from `settings.json`, the habits stay and still shape the dictations; Credential
     Manager lists the entry beside the keys. The next start logs nothing of it.
  2. Telegram has «с маленькой буквы и без точки». In Notepad, take off the full stop alone three times: Notepad's own
     habit «без точки» (its card), and the Telegram row still says «в Telegram». Then in Word, lower case and no full
     stop together three times: the Telegram row says «везде», no card (Notepad's row too: its one form came each time,
     its whole set).
  3. Telegram has «без точки»; add the full stop back three times: the card «Привычка изменилась» with the new rule;
     «Отменить» → «Возвращено», the old row is back and the full stop is never learned there again.
  4. Both themes; reduced motion.
- [ ] **Step 9: report to the owner** what passed, what did not, and ask whether to commit.

---

## Rulings

1. **The app is the history's name for it** ("Telegram Desktop", from the file description), not the exe the spec's
   example shows: the spec says "the dictation's app as the history records it", and the history keeps
   `stored_app(attempt.app)`. It is also the name the section and the rule show. A dictation whose app is not known is
   not counted, and gets only the habits of everywhere.
2. **A habit keeps `forms` (each kind with its detail) instead of one `kind` and one `form` hash.** A merged habit (lower
   case and no full stop) has several forms, and the applied check and widening need their details, not only hashes;
   the hashes are computed from the forms. A replacement's words are always words of the example (a replacement longer
   than 40 characters is no form, and the example falls back to the replacement's own words), so no correction text
   beyond the example reaches disk.
3. **The history entry keeps `habits: [{ id, short }]`, not `[id]`:** "a forgotten habit's chip shows its short name as
   it was" needs the short name on the entry.
4. **`newHabit` is a plain tray kind; the rule goes just before the card on `cue://habit-rule`.** `TrayKind` is `Copy`
   and goes to the page as a bare string; the missing microphone's names and the switched model's name already come
   the same way.
5. **The rule request names the interface language in its instructions;** the user message holds exactly the kind(s),
   the example and the app (the test checks the keys). The model is asked for a rule that starts with the app; on
   widening the "In <app>" / «В <app>» start is cut off, and a rule that does not start so is written anew from the
   template's everywhere form.
6. **Template wording** follows the spec's examples; a scoped habit's template always starts with the app ("В Почта:
   писать «добрый день» вместо «здравствуйте»"), and the spec's replacement example is read as the everywhere form. App
   names are not declined in Russian.
7. **The habits block goes after the cache mark** (after the dictionary block, before the style's block): it changes with
   the app, and the cached prefix stays the one every dictation reads.
8. **"Forget all" also clears the counts (`seen`)**, besides the habits and the declined forms: a form at two would
   otherwise become a habit at the next correction right after the user cleared everything.
9. **A habit's `n` keeps growing** with further corrections of its form in its app (or anywhere, once everywhere), and
   widening adds the other app's count — the row's "how many times".
10. **Only dictations count and take habits.** The watch also runs after a translation (for dictionary words), but a
    translation's correction is not counted, and Translate and "Ask" requests take no habits.
11. **The applied check runs only on a refined text:** a raw fallback marks no habit, though its text may happen to
    show one.
12. **Notices in order:** a habit is "found" when its forms reach three (its number is taken then), though its card can
    open only once its rule came (≤ 5 s). A card waits while the suggestion's capsule is open or shows "undo"; a missed
    suggestion's dot does not hold it back.
13. **"No cleanup connected"** means the chosen connection is a cloud one with no saved key (a local server counts as
    connected). The line is checked when the section is drawn, when the connection changes and when the window gets
    the focus back — not the moment a key is typed on the Keys page (the window has no event for that).
14. **The rule request is not written to usage.csv** (at most one per habit learned, a sentence long).
15. **End mark and first letter need the same last / first word:** one mark swapped for another is neither; a
    correction that shares no word with the paste holds no forms.
16. **"Learn from me" off** drops the habits waiting for their rule and the cards waiting for rest; a card already
    open stays until it closes, and its Undo still forgets.
17. **`RefineContext` gets a hand-written `Debug`** (lengths and a count): it now carries the rules, and its derived
    `Debug` printed the app and the text before the caret.
18. **Hard in the code:** `corrections::changes` cuts a run with more words written than heard to its leading names, so
    "здравствуйте → добрый день" never reached it; the alignment is split out (`corrections::runs`) and habits read the
    raw runs, with the dictionary's behavior unchanged. std's hasher is not stable between versions, so the form hash is
    FNV-1a written out.

### 06.11.2026 addendum

19. **The keyed hash is HMAC-SHA-256, not FNV-1a with the secret mixed in.** Each FNV-1a step can be undone, so the
    hash of a form everyone knows (`digits`, `dash`) gives back the state after the secret, and with it every other
    hash: the secret would protect nothing. HMAC-SHA-256 is written out over `sha2` (already in the workspace; it joins
    `cue-core`), checked against RFC 4231, and cut to its first 8 bytes so `seen` keeps its 16 hex digits. A habit's id
    stays FNV-1a over its forms' hashes, which are keyed already.
20. **The secret lives in `Habits` (`#[serde(skip)]`), and `Form::hash` takes it as a parameter.**
    `SettingChange::HabitForget` declines by hash through `Settings::apply`, which has no other way to reach it. All zero is no secret:
    without one (the system gave no random bytes) nothing is counted, and existing habits still apply. `is_default`
    ignores it, so settings.json never gets an empty `"habits": {}`; `forget_all` keeps it.
21. **The secret is kept as 64 hex digits under `habits-secret` (service "CUE") and read in `Runtime::run`** before the
    first event, beside the Soniox clean-up. A save that fails keeps the new secret for the session (logged); the next
    start then makes another and drops that session's counts again. The leak scan looks for it like a saved key, and
    reads it without ever making one (`kept_habit_secret`).
22. **A new secret makes the habits' ids again** (an id is the hash of keyed hashes). History entries keep the old ids:
    their chips show the short name they carry and never look a habit up, so nothing breaks.
23. **A habit's set is counted under the habit's id**, the spec's "hash of its forms together", in `seen.form` beside
    the forms' hashes; `forget` drops it with the habit. A form counted on a habit of this app or of everywhere belongs
    to no set. Sets may overlap: every habit of another app whose forms all came in the correction counts, and each
    widens at three (a one-form habit's set is its form).
24. **An opposite changes a habit when the new one is learned (`Habits::learn`), not when it is counted:** the spec has
    the opposite count "like any form", which `count` already does, so it only drafts at three as usual. `learn` takes
    out every habit holding an opposite of the draft's forms in the draft's app or everywhere, switched-off ones too
    (two opposites never live together; the spec's "on" changes nothing else, since every correction counts). A habit
    of several forms goes whole when the opposite of one of them is learned (the spec: "the old habit goes"); being
    removed and not declined, its other forms can be learned again.
25. **A habit with an opposite habit in some app never widens**, neither by its set nor by a second draft of its forms
    (that draft only adds to its count): "везде" would put the two together.
26. **"Habit changed" is its own tray kind, `habitChanged`,** a plain string like `newHabit`, with the same look. The
    spec names no done label: it is "Restored" / «Возвращено», since "Undo" brings the old habit back. The habits to
    bring back live on the runtime's card (`HabitCard.replaced`), so its Undo calls `Habits::revert` and writes the
    settings itself instead of going through a `SettingChange`; they come back first in the list, as they were (id, `n`,
    switch).
27. **"Learn from me" turned off with a change waiting for its card** takes the change back (`revert` without declining):
    nothing is learned while it is off, and the replaced habits return.
28. **A merged draft that also replaced habits still gets a card**, "Habit changed"; its Undo removes the merged habit
    and declines its forms (a declined form is in no habit), and brings the replaced ones back.

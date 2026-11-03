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
/// words compared without case and outer punctuation). Deleted and inserted words are not runs; of a run with more new
/// words than old ones, the new side keeps only its leading names and terms.
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
    let starts = |j: usize| j == 0 || b[j - 1].trim_end_matches(['"', '»', '”', ')']).ends_with(SENTENCE_END);
    let (mut i, mut j, mut gi, mut gj) = (0, 0, 0, 0);
    let mut out = Vec::new();
    loop {
        let end = i == a.len() && j == b.len();
        if end || (i < a.len() && j < b.len() && ka[i] == kb[j]) {
            let (old, mut new) = (&a[gi..i], &b[gj..j]);
            // More words written than were heard: only the leading ones that are each a name or a term belong to the
            // word, the rest is text typed on after it ("Марго и скажи" for "марине" is "Марго").
            if new.len() > old.len() {
                let named = new.iter().enumerate().take_while(|&(k, w)| looks_like_name(clean(w), starts(gj + k), corrected));
                new = &new[..named.count()];
            }
            if !old.is_empty() && !new.is_empty() && old.len() <= MAX_RUN && new.len() <= MAX_RUN {
                out.push(Change {
                    heard: clean(&old.join(" ")).to_owned(),
                    word: clean(&new.join(" ")).to_owned(),
                    sentence_start: starts(gj),
                });
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
    fn words_typed_on_after_the_corrected_one_stay_out_of_it() {
        assert_eq!(found("позвони марине", "позвони Марго и скажи"), [pair("марине", "Марго")]);
        assert_eq!(found("открой гит хаб и всё", "открой GitHub и всё"), [pair("гит хаб", "GitHub")]);
        assert_eq!(found("поехали в нью йорк", "поехали в New York завтра"), [pair("нью йорк", "New York")]);
        assert_eq!(found("спроси клод", "спроси Claude Haiku"), [pair("клод", "Claude Haiku")], "each word a name");
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
        assert!(found(&format!("{long}марисоль"), &format!("{long}Marisol")).is_empty());
        let short = "слово ".repeat(10);
        assert_eq!(found(&format!("{short}марисоль"), &format!("{short}Marisol")), [pair("марисоль", "Marisol")]);
    }

    #[test]
    fn debug_shows_no_words() {
        let r = Replacement { heard: "Марисоль".into(), word: "Marisol".into() };
        let d = format!("{r:?}");
        assert!(!d.contains("Marisol") && !d.contains("Марисоль"), "{d}");
    }

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
}

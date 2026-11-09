//! The basics without a model: what CUE knows for certain about a recognized text, fixed before any cleanup.
//!
//! Four steps, in this order, on exact matches only (nothing is guessed): learned words, the spelling of dictionary
//! words, filler words and tidy spacing. See `docs/specs/2026-11-05-no-ai-layer-design.md`. The text a model gets, and
//! the text pasted when no model works, is the result; the history keeps the text as heard.

use crate::refine::{is_term, usable_terms};
use crate::settings::DictionarySettings;

/// Sounds that mean nothing, removed wherever they stand alone ("мм" after a number is millimetres, not a filler).
const FILLERS: [&str; 10] = ["э", "ээ", "эээ", "эм", "мм", "ммм", "um", "uh", "erm", "hmm"];
/// Removed only as the first word of the dictation, and only as a discourse marker: with a comma after it ("Ну, …",
/// "Well, …"). Without one it is a word of the sentence ("Well done", "ну и ладно") and stays.
const OPENERS: [&str; 2] = ["ну", "well"];

/// `raw` with the learned words and the dictionary's spellings applied, fillers removed and spacing tidied. Text that
/// would be left without a letter or a digit stays as it was.
pub fn tidy(raw: &str, dict: &DictionarySettings) -> String {
    let mut learned: Vec<(&str, &str)> = dict
        .learned
        .iter()
        .map(|l| (l.heard.trim(), l.word.trim()))
        .filter(|(heard, word)| is_term(heard) && is_term(word))
        .collect();
    learned.sort_by_key(|(heard, _)| std::cmp::Reverse(size(heard)));
    let mut terms: Vec<(&str, &str)> = usable_terms(&dict.terms).into_iter().map(|t| (t, t)).collect();
    terms.sort_by_key(|(t, _)| std::cmp::Reverse(size(t)));
    // The spellings the first two steps write: a capital never changes the start of one.
    let spellings: Vec<&str> = learned.iter().chain(&terms).map(|&(_, to)| to).collect();

    let text = replace_phrases(raw, &learned);
    let text = replace_phrases(&text, &terms);
    let text = remove_fillers(&text, &spellings);
    let text = remove_opener(&text, &spellings);
    let text = tidy_spacing(&text);
    if text.chars().any(char::is_alphanumeric) { text } else { raw.to_owned() }
}

/// A word of the text: its byte range.
type Span = (usize, usize);

/// A joiner (`-` `'` `’`) between two letters or digits belongs to the word.
fn joins(c: char) -> bool {
    matches!(c, '-' | '\'' | '’')
}

/// The words of `text` (letters, digits, and joiners between them) with their byte ranges.
fn word_spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
            end = i + c.len_utf8();
        } else if !(joins(c) && start.is_some())
            && let Some(s) = start.take()
        {
            spans.push((s, end));
        }
    }
    if let Some(s) = start {
        spans.push((s, end));
    }
    spans
}

/// How many pieces (separated by white space) and characters an entry has: the longer one is tried first.
fn size(entry: &str) -> (usize, usize) {
    (entry.split_whitespace().count(), entry.chars().count())
}

fn same_letter(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// The end of `pieces` found at byte `at` of `text`: each character equal apart from letter case, a run of white space
/// between the pieces. The punctuation of an entry ("C#", "Node.js") has to be in the text as well.
fn match_at(text: &str, at: usize, pieces: &[&str]) -> Option<usize> {
    let mut pos = at;
    for (k, piece) in pieces.iter().enumerate() {
        if k > 0 {
            let rest = &text[pos..];
            let spaces = rest.len() - rest.trim_start().len();
            if spaces == 0 {
                return None;
            }
            pos += spaces;
        }
        for want in piece.chars() {
            let c = text[pos..].chars().next()?;
            if !same_letter(c, want) {
                return None;
            }
            pos += c.len_utf8();
        }
    }
    Some(pos)
}

/// No word goes on in front of byte `at`.
fn word_ends_before(text: &str, at: usize) -> bool {
    let mut before = text[..at].chars().rev();
    match before.next() {
        None => true,
        Some(c) if c.is_alphanumeric() => false,
        Some(c) => !(joins(c) && before.next().is_some_and(char::is_alphanumeric)),
    }
}

/// No word goes on after byte `at`, and an entry's closing sign ("#" of "C#") is not doubled by the text.
fn word_starts_after(text: &str, at: usize) -> bool {
    let last = text[..at].chars().next_back();
    let mut after = text[at..].chars();
    match after.next() {
        None => true,
        Some(c) if Some(c) == last && !c.is_alphanumeric() => false,
        Some(c) if c.is_alphanumeric() => false,
        Some(c) => !(joins(c) && after.next().is_some_and(char::is_alphanumeric)),
    }
}

/// Every entry found whole (equal apart from letter case, on word boundaries) becomes its replacement; at each place
/// the first of `pairs` that fits wins, so the list goes longest first. The result is not searched again.
fn replace_phrases(text: &str, pairs: &[(&str, &str)]) -> String {
    let patterns: Vec<(Vec<&str>, &str)> = pairs
        .iter()
        .map(|&(from, to)| (from.split_whitespace().collect::<Vec<_>>(), to))
        .filter(|(p, _)| !p.is_empty())
        .collect();
    if patterns.is_empty() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        let hit = if word_ends_before(text, i) {
            patterns
                .iter()
                .find_map(|(p, to)| match_at(text, i, p).filter(|&end| word_starts_after(text, end)).map(|end| (end, *to)))
        } else {
            None
        };
        match hit {
            Some((end, to)) => {
                out.push_str(to);
                i = end;
            }
            None => {
                out.push(c);
                i += c.len_utf8();
            }
        }
    }
    out
}

/// The first character at or after byte `from`, if only white space comes before it and it is a small letter, gets its
/// capital, unless a spelling of the dictionary starts there.
fn capitalize_from(text: &mut String, from: usize, spellings: &[&str]) {
    let rest = &text[from..];
    let at = from + rest.len() - rest.trim_start().len();
    if spellings.iter().any(|s| text[at..].starts_with(s)) {
        return;
    }
    if let Some(c) = text[at..].chars().next()
        && c.is_lowercase()
    {
        let upper: String = c.to_uppercase().collect();
        text.replace_range(at..at + c.len_utf8(), &upper);
    }
}

/// "мм" after a number is millimetres.
fn is_unit(text: &str, word: Span) -> bool {
    text[word.0..word.1].to_lowercase() == "мм" && text[..word.0].trim_end().ends_with(|c: char| c.is_ascii_digit())
}

/// Standalone filler words go, with the commas around them; where one stood at the start of a sentence, the next word
/// takes the capital.
fn remove_fillers(text: &str, spellings: &[&str]) -> String {
    let mut text = text.to_owned();
    while let Some((s, e)) = word_spans(&text)
        .into_iter()
        .find(|&w| FILLERS.contains(&text[w.0..w.1].to_lowercase().as_str()) && !is_unit(&text, w))
    {
        let mut from = text[..s].trim_end().len();
        if text[..from].ends_with(',') {
            from -= 1;
        }
        let mut to = e;
        let after = text[e..].trim_start();
        if after.starts_with(',') {
            to = text.len() - after.len() + 1;
        }
        let before = text[..from].trim_end_matches([' ', '\t']);
        let sentence_start = before.is_empty() || before.ends_with(['.', '!', '?', '\n']);
        text.replace_range(from..to, " ");
        if sentence_start {
            capitalize_from(&mut text, from, spellings);
        }
    }
    text
}

/// "ну" or "well" as the first word, a comma after it, goes with its comma; the next word takes the capital. Without
/// the comma the text stays as it is.
fn remove_opener(text: &str, spellings: &[&str]) -> String {
    let lead = text.len() - text.trim_start().len();
    let Some(&(s, e)) = word_spans(text).first() else { return text.to_owned() };
    if s != lead || !OPENERS.contains(&text[s..e].to_lowercase().as_str()) {
        return text.to_owned();
    }
    let Some(rest) = text[e..].trim_start().strip_prefix(',') else { return text.to_owned() };
    let mut rest = rest.to_owned();
    capitalize_from(&mut rest, 0, spellings);
    rest
}

/// A mark that ends a sentence.
fn ends_sentence(c: char) -> bool {
    matches!(c, '.' | '!' | '?')
}

/// No space before `, . ! ? : ;`, single spaces, no doubled commas, no space at the edges. A sentence mark that comes
/// after another one with space between them (what a filler that stood as a sentence of its own leaves: "Привет. Эм.
/// Как дела?") goes; marks written together ("...", "?!") stay.
fn tidy_spacing(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for c in text.chars() {
        if c == ' ' || c == '\t' {
            pending_space = true;
            continue;
        }
        if c == ',' && out.ends_with(',') {
            pending_space = false;
            continue;
        }
        if pending_space && ends_sentence(c) && out.ends_with(ends_sentence) {
            continue;
        }
        let closes = matches!(c, ',' | '.' | '!' | '?' | ':' | ';');
        if pending_space && !out.is_empty() && !out.ends_with('\n') && !closes {
            out.push(' ');
        }
        pending_space = false;
        out.push(c);
    }
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::LearnedWord;

    fn dict(terms: &[&str], learned: &[(&str, &str)]) -> DictionarySettings {
        DictionarySettings {
            terms: terms.iter().map(|t| t.to_string()).collect(),
            learned: learned
                .iter()
                .map(|(heard, word)| LearnedWord { word: word.to_string(), heard: heard.to_string() })
                .collect(),
        }
    }

    fn check(d: &DictionarySettings, cases: &[(&str, &str)]) {
        for (raw, want) in cases {
            assert_eq!(tidy(raw, d), *want, "tidy({raw:?})");
        }
    }

    #[test]
    fn learned_phrase_becomes_word() {
        let d = dict(&[], &[("гит", "Git"), ("гит хаб", "GitHub"), ("claude code", "Claude Code")]);
        check(
            &d,
            &[
                ("Открой гит хаб сейчас", "Открой GitHub сейчас"),
                ("Открой Гит Хаб.", "Открой GitHub."),
                ("гит хаб и гит", "GitHub и Git"),
                ("open claude code now", "open Claude Code now"),
                ("гитара", "гитара"),
            ],
        );
    }

    #[test]
    fn dictionary_case_exact_only() {
        let d = dict(&["GitHub", "Claude Haiku"], &[]);
        check(
            &d,
            &[
                ("push to github", "push to GitHub"),
                ("use claude haiku today", "use Claude Haiku today"),
                ("githubs are here", "githubs are here"),
                ("гитхаб", "гитхаб"),
            ],
        );
    }

    #[test]
    fn fillers_go_with_one_comma() {
        let d = dict(&[], &[]);
        check(
            &d,
            &[
                ("Я, э, приду, эм, завтра.", "Я приду завтра."),
                ("Ну, я приду.", "Я приду."),
                ("Well, I will come.", "I will come."),
                ("I will, um, come.", "I will come."),
                ("Приду, эм.", "Приду."),
                ("Uh hmm, okay then", "Okay then"),
            ],
        );
    }

    #[test]
    fn nu_and_well_only_first_word() {
        let d = dict(&[], &[]);
        check(&d, &[("Он ну пришёл", "Он ну пришёл"), ("It is well known", "It is well known"), ("Он, ну, пришёл", "Он, ну, пришёл")]);
    }

    /// "Ну" and "well" go only as a discourse marker, a comma after them; otherwise they are words of the sentence.
    #[test]
    fn nu_and_well_go_only_with_a_comma() {
        let d = dict(&[], &[]);
        check(
            &d,
            &[
                ("Well done", "Well done"),
                ("well done, team", "well done, team"),
                ("Well, done.", "Done."),
                ("well, I think so", "I think so"),
                ("Well , I think so", "I think so"),
                ("ну и ладно", "ну и ладно"),
                ("Ну что, пойдём?", "Ну что, пойдём?"),
                ("ну, и ладно", "И ладно"),
                ("Ну , пойдём", "Пойдём"),
                ("Ну", "Ну"),
                ("Ну,", "Ну,"),
            ],
        );
    }

    /// A filler said as a sentence of its own leaves no second mark: "Привет. Эм. Как дела?" reads "Привет. Как
    /// дела?". Marks written together stay.
    #[test]
    fn a_filler_sentence_leaves_one_mark() {
        let d = dict(&[], &[]);
        check(
            &d,
            &[
                ("Привет. Эм. Как дела?", "Привет. Как дела?"),
                ("Как дела? Эм. Хорошо.", "Как дела? Хорошо."),
                ("Ok! Um! Go.", "Ok! Go."),
                ("Well. Um. Next.", "Well. Next."),
                ("Привет. Эм.", "Привет."),
                ("Подожди... Эм... Ладно.", "Подожди... Ладно."),
                ("Подожди...", "Подожди..."),
                ("Что?! Правда?", "Что?! Правда?"),
                ("Что?!", "Что?!"),
                ("Привет . Как дела ?", "Привет. Как дела?"),
            ],
        );
    }

    #[test]
    fn whole_words_only() {
        let d = dict(&[], &[]);
        check(
            &d,
            &[("эмоция", "эмоция"), ("humble um", "humble"), ("Нужно ммм подумать", "Нужно подумать"), ("umbrella", "umbrella")],
        );
    }

    #[test]
    fn tidy_spaces_and_commas() {
        let d = dict(&[], &[]);
        check(
            &d,
            &[
                ("Привет , мир  !  Как дела ?", "Привет, мир! Как дела?"),
                ("a,, b ;c", "a, b;c"),
                ("  one   two  ", "one two"),
                ("слово : ещё", "слово: ещё"),
            ],
        );
    }

    #[test]
    fn capital_after_removed_first_word() {
        let d = dict(&[], &[]);
        check(&d, &[("эм, привет", "Привет"), ("ну, привет", "Привет"), ("um okay", "Okay"), ("и эм привет", "и привет")]);
    }

    #[test]
    fn empty_result_keeps_raw() {
        let d = dict(&[], &[]);
        check(&d, &[("эм", "эм"), ("Um.", "Um."), ("", "")]);
    }

    #[test]
    fn entries_with_punctuation_match_whole() {
        let d = dict(&["C#", "C++", "Node.js"], &[("си шарп", "C#")]);
        check(
            &d,
            &[
                ("I like c and C# and c++ and node.js.", "I like c and C# and C++ and Node.js."),
                ("c# is fine", "C# is fine"),
                ("C# C++ Node.js", "C# C++ Node.js"),
                ("a c b", "a c b"),
                ("node js", "node js"),
                ("напиши на си шарп", "напиши на C#"),
                ("c##", "c##"),
            ],
        );
    }

    #[test]
    fn capital_never_recases_a_spelling_of_the_dictionary() {
        let d = dict(&["iPhone"], &[("эй пад", "iPad")]);
        check(
            &d,
            &[
                ("эм iphone", "iPhone"),
                ("ну, iphone купил", "iPhone купил"),
                ("эм, эй пад", "iPad"),
                ("Привет. Эм, iphone", "Привет. iPhone"),
            ],
        );
    }

    #[test]
    fn mm_after_a_number_is_a_unit() {
        let d = dict(&[], &[]);
        check(
            &d,
            &[("5 мм", "5 мм"), ("размер 10 мм, э, ок", "размер 10 мм ок"), ("Привет мм ок", "Привет ок"), ("5, мм, ок", "5 ок")],
        );
    }

    #[test]
    fn filler_at_a_sentence_start_restores_the_capital() {
        let d = dict(&[], &[]);
        check(
            &d,
            &[
                ("Привет. Эм, как дела?", "Привет. Как дела?"),
                ("Ok! um what now", "Ok! What now"),
                ("Привет, эм, как дела?", "Привет как дела?"),
                ("Он пришёл эм и ушёл", "Он пришёл и ушёл"),
            ],
        );
    }
}

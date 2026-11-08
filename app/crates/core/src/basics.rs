//! The basics without a model: what CUE knows for certain about a recognized text, fixed before any cleanup.
//!
//! Four steps, in this order, on exact matches only (nothing is guessed): learned words, the spelling of dictionary
//! words, filler words and tidy spacing. See `docs/specs/2026-11-05-no-ai-layer-design.md`. The text a model gets, and
//! the text pasted when no model works, is the result; the history keeps the text as heard.

use crate::refine::{is_term, usable_terms};
use crate::settings::DictionarySettings;

/// Sounds that mean nothing, removed wherever they stand alone.
const FILLERS: [&str; 10] = ["э", "ээ", "эээ", "эм", "мм", "ммм", "um", "uh", "erm", "hmm"];
/// Removed only as the first word of the dictation.
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
    learned.sort_by_key(|(heard, _)| std::cmp::Reverse(words(heard).len()));
    let mut terms: Vec<(&str, &str)> = usable_terms(&dict.terms).into_iter().map(|t| (t, t)).collect();
    terms.sort_by_key(|(t, _)| std::cmp::Reverse(words(t).len()));

    let text = replace_phrases(raw, &learned);
    let text = replace_phrases(&text, &terms);
    let (text, mut capital) = remove_fillers(&text);
    let (text, opener) = remove_opener(&text);
    capital |= opener;
    let text = tidy_spacing(&text);
    if !text.chars().any(char::is_alphanumeric) {
        return raw.to_owned();
    }
    if capital { capitalize(&text) } else { text }
}

/// A word of the text: its byte range.
type Span = (usize, usize);

/// The words of `text` (letters, digits, and `-` `'` `’` between them) with their byte ranges.
fn word_spans(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    for (i, c) in text.char_indices() {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
            end = i + c.len_utf8();
        } else if !(matches!(c, '-' | '\'' | '’') && start.is_some())
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

/// The lowercase words of a phrase.
fn words(phrase: &str) -> Vec<String> {
    word_spans(phrase).into_iter().map(|(s, e)| phrase[s..e].to_lowercase()).collect()
}

/// Every phrase found as whole words (case-insensitive, only white space between them) becomes its replacement; at
/// each place the first of `pairs` that fits wins, so the list goes longest first. The result is not searched again.
fn replace_phrases(text: &str, pairs: &[(&str, &str)]) -> String {
    if pairs.is_empty() {
        return text.to_owned();
    }
    let spans = word_spans(text);
    let lower: Vec<String> = spans.iter().map(|&(s, e)| text[s..e].to_lowercase()).collect();
    let patterns: Vec<(Vec<String>, &str)> = pairs.iter().map(|&(from, to)| (words(from), to)).collect();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < spans.len() {
        let hit = patterns.iter().find(|(p, _)| {
            !p.is_empty()
                && i + p.len() <= spans.len()
                && p.iter().zip(&lower[i..]).all(|(a, b)| a == b)
                && (i..i + p.len() - 1).all(|k| text[spans[k].1..spans[k + 1].0].chars().all(char::is_whitespace))
        });
        match hit {
            Some((p, to)) => {
                out.push_str(&text[copied..spans[i].0]);
                out.push_str(to);
                copied = spans[i + p.len() - 1].1;
                i += p.len();
            }
            None => i += 1,
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// Standalone filler words go, with the commas around them. True when the text then starts where a filler was.
fn remove_fillers(text: &str) -> (String, bool) {
    let mut text = text.to_owned();
    let mut at_start = false;
    while let Some((s, e)) =
        word_spans(&text).into_iter().find(|&(s, e)| FILLERS.contains(&text[s..e].to_lowercase().as_str()))
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
        at_start |= text[..from].trim().is_empty();
        text.replace_range(from..to, " ");
    }
    (text, at_start)
}

/// "ну" or "well" as the first word goes, with its comma. True when it went.
fn remove_opener(text: &str) -> (String, bool) {
    let lead = text.len() - text.trim_start().len();
    let Some(&(s, e)) = word_spans(text).first() else { return (text.to_owned(), false) };
    if s != lead || !OPENERS.contains(&text[s..e].to_lowercase().as_str()) {
        return (text.to_owned(), false);
    }
    let rest = text[e..].trim_start();
    let rest = rest.strip_prefix(',').unwrap_or(rest);
    (rest.to_owned(), true)
}

/// No space before `, . ! ? : ;`, single spaces, no doubled commas, no space at the edges.
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
        let closes = matches!(c, ',' | '.' | '!' | '?' | ':' | ';');
        if pending_space && !out.is_empty() && !out.ends_with('\n') && !closes {
            out.push(' ');
        }
        pending_space = false;
        out.push(c);
    }
    out.trim().to_owned()
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
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
        check(&d, &[("Он ну пришёл", "Он ну пришёл"), ("It is well known", "It is well known")]);
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
        check(&d, &[("эм, привет", "Привет"), ("ну привет", "Привет"), ("um okay", "Okay"), ("и эм привет", "и привет")]);
    }

    #[test]
    fn empty_result_keeps_raw() {
        let d = dict(&[], &[]);
        check(&d, &[("эм", "эм"), ("Um.", "Um."), ("", "")]);
    }
}

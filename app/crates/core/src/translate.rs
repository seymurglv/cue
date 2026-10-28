//! Translation (spec 2 §5): the languages Soniox recognizes, the language of the speech, which way to translate, and the
//! check of the model's translation. Pure logic.

/// Every language Soniox recognizes (soniox.com/docs/stt/concepts/supported-languages, checked 28.10.2026):
/// ISO 639-1 code and English name. The name goes into the translation instruction for the model.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("af", "Afrikaans"), ("sq", "Albanian"), ("ar", "Arabic"), ("az", "Azerbaijani"), ("eu", "Basque"), ("be", "Belarusian"),
    ("bn", "Bengali"), ("bs", "Bosnian"), ("bg", "Bulgarian"), ("ca", "Catalan"), ("zh", "Chinese"), ("hr", "Croatian"),
    ("cs", "Czech"), ("da", "Danish"), ("nl", "Dutch"), ("en", "English"), ("et", "Estonian"), ("fi", "Finnish"),
    ("fr", "French"), ("gl", "Galician"), ("de", "German"), ("el", "Greek"), ("gu", "Gujarati"), ("he", "Hebrew"),
    ("hi", "Hindi"), ("hu", "Hungarian"), ("id", "Indonesian"), ("it", "Italian"), ("ja", "Japanese"), ("kn", "Kannada"),
    ("kk", "Kazakh"), ("ko", "Korean"), ("lv", "Latvian"), ("lt", "Lithuanian"), ("mk", "Macedonian"), ("ms", "Malay"),
    ("ml", "Malayalam"), ("mr", "Marathi"), ("no", "Norwegian"), ("fa", "Persian"), ("pl", "Polish"), ("pt", "Portuguese"),
    ("pa", "Punjabi"), ("ro", "Romanian"), ("ru", "Russian"), ("sr", "Serbian"), ("sk", "Slovak"), ("sl", "Slovenian"),
    ("es", "Spanish"), ("sw", "Swahili"), ("sv", "Swedish"), ("tl", "Tagalog"), ("ta", "Tamil"), ("te", "Telugu"),
    ("th", "Thai"), ("tr", "Turkish"), ("uk", "Ukrainian"), ("ur", "Urdu"), ("vi", "Vietnamese"), ("cy", "Welsh"),
];

/// Codes Windows (or a person) may write differently from Soniox.
const ALIASES: &[(&str, &str)] = &[("nb", "no"), ("nn", "no"), ("fil", "tl"), ("iw", "he"), ("in", "id")];

/// The Soniox code for `code`: trimmed, lowercase, the first subtag of a locale name ("ru-RU", "zh-Hans-CN"), aliases
/// applied. `None` for a language Soniox does not recognize.
pub fn known(code: &str) -> Option<&'static str> {
    let primary = code.trim().split(['-', '_']).next()?.to_ascii_lowercase();
    let primary = ALIASES.iter().find(|(a, _)| *a == primary).map_or(primary.as_str(), |(_, to)| to);
    LANGUAGES.iter().find(|(c, _)| *c == primary).map(|(c, _)| *c)
}

/// The English name of a code, for the instruction to the model. Codes come through [`known`]; an unknown one (never
/// expected) still gives a usable instruction.
pub fn name(code: &str) -> &'static str {
    LANGUAGES.iter().find(|(c, _)| *c == code).map_or("the target language", |(_, n)| n)
}

/// Words of the final tokens per language: the speech language is the one with the most words (spec 2 §5 — "by the
/// language of most words"). Tokens come in order and are pieces of words: a word starts at the first letter after
/// whitespace (or the first letter of all) and belongs to the language of the token it starts in, so a word split over
/// several tokens counts once, and a short phrase is not outweighed by one long foreign name. Punctuation starts no
/// word. In scripts written without spaces (Chinese, Japanese, Thai…) every character counts as a word. Tokens without
/// a known language are skipped.
#[derive(Debug, Default, Clone)]
pub struct LanguageTally {
    words: Vec<(&'static str, usize)>,
    /// The last letter had no whitespace after it yet: the next letter continues its word.
    in_word: bool,
}

impl LanguageTally {
    pub fn add(&mut self, language: &str, text: &str) {
        let Some(code) = known(language) else { return };
        let mut n = 0;
        for c in text.chars() {
            if c.is_whitespace() {
                self.in_word = false;
            } else if c.is_alphabetic() {
                if unspaced(c) {
                    n += 1;
                    self.in_word = false;
                } else if !self.in_word {
                    n += 1;
                    self.in_word = true;
                }
            }
        }
        if n == 0 {
            return;
        }
        match self.words.iter_mut().find(|(c, _)| *c == code) {
            Some((_, count)) => *count += n,
            None => self.words.push((code, n)),
        }
    }

    /// The language with the most words; on a tie, the one heard first.
    pub fn dominant(&self) -> Option<&'static str> {
        self.words
            .iter()
            .fold(None, |best: Option<(&'static str, usize)>, &(c, n)| match best {
                Some((_, m)) if m >= n => best,
                _ => Some((c, n)),
            })
            .map(|(c, _)| c)
    }
}

/// Scripts written without spaces between words, where a word boundary cannot be seen: Han, Hiragana, Katakana, Thai,
/// Lao, Myanmar, Khmer.
fn unspaced(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0E00..=0x0EFF // Thai, Lao
            | 0x1000..=0x109F // Myanmar
            | 0x1780..=0x17FF // Khmer
            | 0x3040..=0x30FF // Hiragana, Katakana
            | 0x31F0..=0x31FF // Katakana phonetic extensions
            | 0x3400..=0x4DBF // CJK extension A
            | 0x4E00..=0x9FFF // CJK unified ideographs
            | 0xF900..=0xFAFF // CJK compatibility ideographs
            | 0xFF66..=0xFF9F // halfwidth Katakana
            | 0x20000..=0x323AF // CJK extensions B–H
    )
}

/// The language to write the result in (spec 2 §5): speech not in the target language goes to the target; speech in the
/// target goes to the home language. Unknown speech goes to the target; with no home language, target speech stays in
/// the target (it is only cleaned up).
pub fn output_language(speech: Option<&str>, target: &'static str, home: Option<&'static str>) -> &'static str {
    match (speech, home) {
        (Some(s), Some(h)) if s == target => h,
        _ => target,
    }
}

/// The home language: `translate.home` from the settings; else the Windows display language; else the first Soniox
/// language hint. One equal to the target is skipped at every step: a pair needs two languages.
pub fn resolve_home(setting: Option<&str>, windows: Option<&str>, hints: &[String], target: &str) -> Option<&'static str> {
    setting
        .and_then(known)
        .filter(|c| *c != target)
        .or_else(|| windows.and_then(known).filter(|c| *c != target))
        .or_else(|| hints.iter().filter_map(|h| known(h)).find(|c| *c != target))
}

/// The instruction for one translated dictation: a system block after the cached prompt (spec 2 §5), so the cache is
/// shared with plain dictation. It overrides the prompt's "never translate" and "the language it was spoken in".
pub fn instruction(to: &str) -> String {
    let lang = name(to);
    format!(
        "Translation is on for this dictation. Clean up the transcript by the rules above, then write the whole result in {lang}: \
         translate every part that is not in {lang} yet. This replaces the rules to keep the spoken language and never translate. \
         Keep the meaning, the tone and the formatting rules above (plain text, the list rules, one line in terminals). \
         Names and terms from the user's list, product names and code stay as written. Return only the {lang} text in the `text` field."
    )
}

/// Translation takes a second longer than a cleanup (spec 2 §5: 4 s against 3 s).
pub fn timeout_ms(raw: &str) -> u64 {
    crate::refine::timeout_ms(raw) + 1_000
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Script {
    Latin,
    Cyrillic,
    Greek,
    Arabic,
    Hebrew,
    Devanagari,
    Bengali,
    Gurmukhi,
    Gujarati,
    Tamil,
    Telugu,
    Kannada,
    Malayalam,
    Thai,
    Han,
    Hangul,
}

fn script_of(c: char) -> Option<Script> {
    use Script::*;
    Some(match c as u32 {
        0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Latin,
        0x370..=0x3FF | 0x1F00..=0x1FFF => Greek,
        0x400..=0x52F => Cyrillic,
        0x590..=0x5FF => Hebrew,
        0x600..=0x6FF | 0x750..=0x77F | 0x8A0..=0x8FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Arabic,
        0x900..=0x97F => Devanagari,
        0x980..=0x9FF => Bengali,
        0xA00..=0xA7F => Gurmukhi,
        0xA80..=0xAFF => Gujarati,
        0xB80..=0xBFF => Tamil,
        0xC00..=0xC7F => Telugu,
        0xC80..=0xCFF => Kannada,
        0xD00..=0xD7F => Malayalam,
        0xE00..=0xE7F => Thai,
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => Hangul,
        // Kana goes with Han: Japanese writes both, and only Japanese and Chinese use them.
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => Han,
        _ => return None,
    })
}

/// The script a language is written in; `None` for Serbian (both Cyrillic and Latin are usual).
fn script_of_language(code: &str) -> Option<Script> {
    use Script::*;
    Some(match code {
        "be" | "bg" | "kk" | "mk" | "ru" | "uk" => Cyrillic,
        "sr" => return None,
        "el" => Greek,
        "ar" | "fa" | "ur" => Arabic,
        "he" => Hebrew,
        "hi" | "mr" => Devanagari,
        "bn" => Bengali,
        "pa" => Gurmukhi,
        "gu" => Gujarati,
        "ta" => Tamil,
        "te" => Telugu,
        "kn" => Kannada,
        "ml" => Malayalam,
        "th" => Thai,
        "zh" | "ja" => Han,
        "ko" => Hangul,
        _ => Latin,
    })
}

/// How many Latin letters one letter of the script is worth in length: Han and Hangul pack a word into fewer characters.
fn density(s: Option<Script>) -> f32 {
    match s {
        Some(Script::Han) => 3.0,
        Some(Script::Hangul) => 2.0,
        _ => 1.0,
    }
}

/// The share of the result's letters in the target script, from which the result counts as written in it: names,
/// product names and code stay as said.
const MIN_TARGET_SCRIPT: f32 = 0.6;
/// Length bounds of the translation against the speech, in weighted letters ([`density`]); checked from
/// `MIN_LETTERS_FOR_RATIO` letters of speech: a short phrase legitimately changes length more.
const MIN_RATIO: f32 = 0.35;
const MAX_RATIO: f32 = 2.8;
const MIN_LETTERS_FOR_RATIO: usize = 20;

/// The translation can be pasted (spec 2 §5): not empty; its length is within bounds of what was said; when the speech
/// and the target language use different scripts, the result is written in the target one. The word overlap check of
/// dictation does not apply: a translation shares no words with the speech.
pub fn translation_ok(raw: &str, out: &str, to: &str) -> bool {
    let out = out.trim();
    if out.is_empty() {
        return false;
    }
    let letters = |t: &str| t.chars().filter(|c| c.is_alphabetic()).collect::<Vec<_>>();
    let (src, dst) = (letters(raw), letters(out));
    if dst.is_empty() {
        return false;
    }
    let src_script = dominant_script(&src);
    let to_script = script_of_language(to);
    if src.len() >= MIN_LETTERS_FOR_RATIO {
        let ratio = dst.len() as f32 * density(to_script) / (src.len() as f32 * density(src_script));
        if !(MIN_RATIO..=MAX_RATIO).contains(&ratio) {
            return false;
        }
    }
    match to_script {
        Some(t) if src_script != Some(t) => {
            let in_target = dst.iter().filter(|c| script_of(**c) == Some(t)).count();
            in_target as f32 / dst.len() as f32 >= MIN_TARGET_SCRIPT
        }
        _ => true,
    }
}

/// The script most letters are written in; `None` when no letter belongs to a known script.
fn dominant_script(letters: &[char]) -> Option<Script> {
    let mut counts: Vec<(Script, usize)> = Vec::new();
    for s in letters.iter().filter_map(|c| script_of(*c)) {
        match counts.iter_mut().find(|(x, _)| *x == s) {
            Some((_, n)) => *n += 1,
            None => counts.push((s, 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(s, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_normalizes_codes() {
        assert_eq!(known(" RU "), Some("ru"));
        assert_eq!(known("ru-RU"), Some("ru"));
        assert_eq!(known("zh-Hans-CN"), Some("zh"));
        assert_eq!(known("nb-NO"), Some("no"));
        assert_eq!(known("fil-PH"), Some("tl"));
        assert_eq!(known("xx"), None);
        assert_eq!(known(""), None);
        assert_eq!(LANGUAGES.len(), 60);
    }

    #[test]
    fn every_language_is_known_by_its_code_and_named() {
        for &(code, english) in LANGUAGES {
            assert_eq!(known(code), Some(code), "{code}");
            assert_eq!(name(code), english, "{code}");
        }
        for (alias, code) in [("nn", "no"), ("iw-IL", "he"), ("in", "id"), ("FIL", "tl")] {
            assert_eq!(known(alias), Some(code), "{alias}");
        }
        assert_eq!(name("xx"), "the target language");
    }

    /// Tokens as Soniox gives them: one per word or piece of a word, a word's first token with its leading space.
    fn tally(tokens: &[(&str, &str)]) -> Option<&'static str> {
        let mut t = LanguageTally::default();
        for (language, text) in tokens {
            t.add(language, text);
        }
        t.dominant()
    }

    #[test]
    fn tally_picks_the_language_with_most_words() {
        assert_eq!(tally(&[("ru", "Скажи"), ("en", " Soniox"), ("ru", " что всё готово")]), Some("ru"));
        assert_eq!(tally(&[("en", "ok"), ("ru", " ок")]), Some("en"), "a tie goes to the language heard first");
        assert_eq!(tally(&[("en", "ok"), ("ru", " ок"), ("xx", " a b c d")]), Some("en"), "unknown codes are skipped");
        assert_eq!(LanguageTally::default().dominant(), None);
    }

    /// A short phrase with a long foreign name: by letters it would be English and go untranslated.
    #[test]
    fn a_foreign_name_does_not_outweigh_a_word() {
        assert_eq!(tally(&[("ru", "Открой"), ("en", " Telegram")]), Some("ru"));
        assert_eq!(tally(&[("ru", "Напиши"), ("en", " Alexander")]), Some("ru"));
    }

    #[test]
    fn a_word_split_across_tokens_counts_once() {
        let tokens = [
            ("ru", "Да"), ("ru", " нет"), ("ru", " да"),
            ("en", " Tele"), ("en", "gram"), ("en", " web"), ("en", "site"),
        ];
        assert_eq!(tally(&tokens), Some("ru"), "ru 3 words, en 2");
        // The word belongs to the language of the token it starts in.
        assert_eq!(tally(&[("en", "Tele"), ("ru", "грам"), ("ru", " да")]), Some("en"), "en 1, ru 1: a tie");
    }

    #[test]
    fn scripts_without_spaces_count_every_character() {
        assert_eq!(tally(&[("ru", "Привет"), ("zh", "你好")]), Some("zh"), "ru 1 word, zh 2 characters");
        assert_eq!(tally(&[("en", "Tokyo"), ("en", " station"), ("ja", " 東京に")]), Some("ja"));
        assert_eq!(tally(&[("en", "in"), ("en", " English"), ("th", " ไทย")]), Some("th"));
    }

    #[test]
    fn punctuation_counts_nothing() {
        assert_eq!(tally(&[("en", "?"), ("en", " —"), ("en", " ...")]), None);
        assert_eq!(tally(&[("ru", "Да"), ("en", ","), ("en", " !"), ("en", " ?")]), Some("ru"));
        // Punctuation between two pieces does not split a word; whitespace after it does start one.
        assert_eq!(tally(&[("en", "Wi"), ("en", "-"), ("en", "Fi"), ("ru", " да"), ("ru", ", "), ("ru", "нет")]), Some("ru"));
    }

    #[test]
    fn direction_follows_the_speech() {
        assert_eq!(output_language(Some("ru"), "en", Some("ru")), "en");
        assert_eq!(output_language(Some("en"), "en", Some("ru")), "ru");
        assert_eq!(output_language(Some("de"), "en", Some("ru")), "en", "any other language goes to the target");
        assert_eq!(output_language(None, "en", Some("ru")), "en");
        assert_eq!(output_language(Some("en"), "en", None), "en");
    }

    #[test]
    fn home_falls_back_and_never_equals_the_target() {
        let hints = vec!["ru".to_owned(), "en".to_owned()];
        assert_eq!(resolve_home(Some("de"), Some("ru-RU"), &hints, "en"), Some("de"));
        assert_eq!(resolve_home(None, Some("ru-RU"), &hints, "en"), Some("ru"));
        assert_eq!(resolve_home(None, Some("en-US"), &hints, "en"), Some("ru"), "Windows in the target language: the first other hint");
        assert_eq!(resolve_home(Some("en"), None, &[], "en"), None);
        assert_eq!(resolve_home(Some("klingon"), Some("fr-FR"), &hints, "en"), Some("fr"));
    }

    #[test]
    fn translation_in_the_target_script_passes() {
        assert!(translation_ok("напомни мне завтра позвонить Ане в десять", "Remind me to call Anya at ten tomorrow.", "en"));
        assert!(translation_ok("remind me to call Anya tomorrow at ten", "Напомни мне завтра в десять позвонить Ане.", "ru"));
        assert!(translation_ok("открой Soniox и проверь ключ", "Open Soniox and check the key.", "en"));
    }

    #[test]
    fn untranslated_text_fails_for_a_pair_with_another_script() {
        assert!(!translation_ok("напомни мне завтра позвонить Ане", "Напомни мне завтра позвонить Ане.", "en"));
        assert!(!translation_ok("call Anya tomorrow at ten please", "Call Anya tomorrow at ten, please.", "ru"));
    }

    #[test]
    fn an_answer_instead_of_a_translation_fails_on_length() {
        let raw = "какая сегодня погода в Москве";
        let answer = "I can't check the weather, but you can open a weather site or app and search for Moscow to see the current forecast and temperature for today.";
        assert!(!translation_ok(raw, answer, "en"));
    }

    #[test]
    fn same_script_pairs_check_only_length() {
        assert!(translation_ok("call me when you are home", "Ruf mich an, wenn du zu Hause bist.", "de"));
        assert!(translation_ok("短い", "Short.", "en"), "short speech: no length check");
        assert!(translation_ok("我明天早上十点给你打电话然后我们讨论这个计划的细节", "I'll call you tomorrow at ten in the morning and we'll go over the details of this plan.", "en"));
        assert!(!translation_ok("hello", "   ", "ru"));
    }

    #[test]
    fn the_instruction_names_the_language() {
        let text = instruction("en");
        assert!(text.contains("English") && text.contains("text"), "{text}");
        assert!(!text.contains("{lang}"), "{text}");
        assert!(instruction("ru").contains("Russian"));
    }

    #[test]
    fn translation_gets_a_second_more_than_a_cleanup() {
        assert_eq!(timeout_ms("a b"), 4_000);
        assert_eq!(timeout_ms("a b"), crate::refine::timeout_ms("a b") + 1_000);
    }
}

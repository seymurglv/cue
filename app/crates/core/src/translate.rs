//! Translation (spec 2 §5): the languages a translation can go to, which way to translate, and the
//! check of the model's translation. Pure logic.
use std::collections::HashSet;

/// Every language a translation can go to (the list CUE has used since its first version): ISO 639-1 code and English
/// name. The name goes into the translation instruction for the model.
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

/// Codes Windows (or a person) may write differently.
const ALIASES: &[(&str, &str)] = &[("nb", "no"), ("nn", "no"), ("fil", "tl"), ("iw", "he"), ("in", "id")];

/// CUE's code for `code`: trimmed, lowercase, the first subtag of a locale name ("ru-RU", "zh-Hans-CN"), aliases
/// applied. `None` for a language not in [`LANGUAGES`].
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

/// The language to write the result in (spec 2 §5): speech not in the target language goes to the target; speech in the
/// target goes to the home language. Unknown speech goes to the target; with no home language, target speech stays in
/// the target (it is only cleaned up).
pub fn output_language(speech: Option<&str>, target: &'static str, home: Option<&'static str>) -> &'static str {
    match (speech, home) {
        (Some(s), Some(h)) if s == target => h,
        _ => target,
    }
}

/// The home language: `translate.home` from the settings; else the Windows display language; else the first
/// language of `hints`. One equal to the target is skipped at every step: a pair needs two languages.
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
pub(crate) enum Script {
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

pub(crate) fn script_of(c: char) -> Option<Script> {
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
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF | 0xFFA0..=0xFFDC => Hangul,
        // Kana goes with Han: Japanese writes both, and only Japanese and Chinese use them. Full- and halfwidth.
        _ if is_kana(c) => Han,
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x323AF => Han,
        _ => return None,
    })
}

/// Hiragana and Katakana, full- and halfwidth: in the script check they are Han ([`script_of`]), in length a syllable
/// ([`density`]).
fn is_kana(c: char) -> bool {
    matches!(u32::from(c), 0x3040..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F)
}

/// The script a language is written in; `None` for Serbian (both Cyrillic and Latin are usual).
pub(crate) fn script_of_language(code: &str) -> Option<Script> {
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

/// How many Latin letters one letter is worth: a Han character is a word or most of one, a Hangul block a syllable of
/// two or three letters, a kana a syllable of one or two ("か" is "ka"). Every letter weighs by its own script, in the
/// length check ([`weighted`]) and in the script share ([`written_in`]).
fn density(c: char) -> f32 {
    if is_kana(c) {
        return 1.5;
    }
    match script_of(c) {
        Some(Script::Han) => 3.0,
        Some(Script::Hangul) => 2.0,
        _ => 1.0,
    }
}

/// The weight of the letters, each by its own script ([`density`]): Latin code inside Chinese speech weighs as Latin.
fn weighted(letters: &[char]) -> f32 {
    letters.iter().copied().map(density).sum()
}

/// How many letters one digit is worth in the length check, in the speech and in the result alike. A translation often
/// writes the speech's number words as digits, and recognition writes some numbers in digits itself: "sixty thousand" (13
/// letters) becomes "60 000" (5 digits), "three thirty" (11) "15:30" (4), "сто двадцать пять" (15) "125" (3). Two and a
/// half letters a digit is at the low end of that, enough that a sentence with its numbers in digits stays above the
/// lower bound. A number usually weighs no more in digits than in the words it stands for, but not always: a large
/// round number said in one word ("миллиард", 8 letters) and written in many digits ("1 000 000 000", 10 digits, 25)
/// can push a short result over the upper bound, and that translation then fails safely (Retry).
const DIGIT_WEIGHT: f32 = 2.5;

/// The length of a text for the length check: its letters ([`weighted`]) and its digits ([`DIGIT_WEIGHT`]).
fn size(letters: &[char], text: &str) -> f32 {
    weighted(letters) + digits(text) as f32 * DIGIT_WEIGHT
}

/// Digits of any script (0–9, Arabic-Indic, Devanagari, fullwidth…) that are not letters.
fn digits(text: &str) -> usize {
    text.chars().filter(|c| c.is_numeric() && !c.is_alphabetic()).count()
}

/// The share of the counted letters of the result ([`written_in`], weighted by [`density`]) in the target script, from
/// which the result counts as written in it. A cleanup that respells a few names in the target script stays below it.
const MIN_TARGET_SCRIPT: f32 = 0.8;
/// Length bounds of the translation against the speech, in weighted letters and digits ([`size`]); checked from
/// `MIN_LETTERS_FOR_RATIO` letters of speech: a short phrase legitimately changes length more. A result of digits alone
/// is checked however short the speech: its length is all there is to check.
const MIN_RATIO: f32 = 0.35;
const MAX_RATIO: f32 = 2.8;
const MIN_LETTERS_FOR_RATIO: usize = 20;

/// The translation can be pasted (spec 2 §5: untranslated text is never pasted silently): it has letters or digits; its
/// length is within bounds of what was said; when the speech is in another script than the target language, the result
/// is written in the target script ([`written_in`]). Latin is the script of names, product names and code in any
/// language, so the speech's script is its main non-Latin one when it has any ([`speech_script`]). A pair in one script
/// (English into German) is checked by length only: there the script cannot tell a translation from a cleanup. A result
/// of digits alone ("сто двадцать пять" → "125") is checked by length only too: no untranslated word can be pasted. The
/// word overlap check of dictation does not apply: a translation shares no words with the speech. `terms` is the user's
/// dictionary (`dictionary.terms`).
/// Known limit: a short question answered with a bare number ("какой год" → "2026") passes the digits-only path, as a
/// worded answer to a short question passes the script check.
pub fn translation_ok(raw: &str, out: &str, terms: &[String], to: &str) -> bool {
    let letters = |t: &str| t.chars().filter(|c| c.is_alphabetic()).collect::<Vec<_>>();
    let (src, dst) = (letters(raw), letters(out));
    let digits_only = dst.is_empty();
    if digits_only && digits(out) == 0 {
        return false;
    }
    if src.len() >= MIN_LETTERS_FOR_RATIO || digits_only {
        let ratio = size(&dst, out) / size(&src, raw);
        if !(MIN_RATIO..=MAX_RATIO).contains(&ratio) {
            return false;
        }
    }
    match script_of_language(to) {
        Some(t) if !digits_only && speech_script(&src) != Some(t) => written_in(t, raw, out, terms),
        _ => true,
    }
}

/// The result is written in the `target` script. Only words that can show a translation count: never the words of the
/// dictionary, in any script (the model writes them as listed); never a word in the target script that the speech
/// already has (names, product names and code kept as said: "запусти npm install" → "Run npm install." counts only
/// "Run"); and into a non-Latin target no Latin word (names, product names and code stay in Latin). Words are compared
/// by [`script_words`], ignoring case. Of the counted words at least one is in the target script, and at least
/// [`MIN_TARGET_SCRIPT`] of their letters, weighted by [`density`], are. An echo of the speech, code-heavy speech
/// included, has no counted word in the target script and fails; so does a cleanup that respells a few names in Latin.
///
/// Known limits: into a non-Latin target Latin text shows nothing either way, so the English original followed by its
/// translation passes, and so does English with one word in the target script; a phrase of names alone around one word
/// of the speech ("кубернетес и докер" → "Kubernetes и Docker.") passes into English unless the names are listed; a
/// translation made only of dictionary terms ("Sberbank" → "Сбербанк" with that term listed) fails.
/// A translation made only of target-script words the speech already has fails ("сделай git push" → "Git push."):
/// nothing in it shows a translation.
fn written_in(target: Script, raw: &str, out: &str, terms: &[String]) -> bool {
    let lowercase = |words: Vec<(Option<Script>, String)>| words.into_iter().map(|(_, w)| w.to_lowercase());
    let listed: HashSet<String> =
        crate::refine::usable_terms(terms).into_iter().flat_map(|t| lowercase(script_words(t))).collect();
    let spoken: HashSet<String> = lowercase(script_words(raw)).collect();
    let counted: Vec<(Option<Script>, String)> = script_words(out)
        .into_iter()
        .filter(|(s, w)| {
            let w = w.to_lowercase();
            let kept_name = target != Script::Latin && *s == Some(Script::Latin);
            let kept_from_speech = *s == Some(target) && spoken.contains(&w);
            !kept_name && !kept_from_speech && !listed.contains(&w)
        })
        .collect();
    if !counted.iter().any(|(s, _)| *s == Some(target)) {
        return false;
    }
    let weight = |(_, w): &(Option<Script>, String)| w.chars().map(density).sum::<f32>();
    let all: f32 = counted.iter().map(weight).sum();
    let in_target: f32 = counted.iter().filter(|(s, _)| *s == Some(target)).map(weight).sum();
    in_target / all >= MIN_TARGET_SCRIPT
}

/// Words for the script check: maximal runs of letters of one script, so "检查CUE的GitHub" is four words and a name
/// glued to text in another script is still a word of its own.
fn script_words(text: &str) -> Vec<(Option<Script>, String)> {
    let mut words: Vec<(Option<Script>, String)> = Vec::new();
    let mut in_word = false;
    for c in text.chars() {
        if !c.is_alphabetic() {
            in_word = false;
            continue;
        }
        let s = script_of(c);
        match words.last_mut() {
            Some((ws, w)) if in_word && *ws == s => w.push(c),
            _ => words.push((s, c.to_string())),
        }
        in_word = true;
    }
    words
}

/// The script of the speech: its main non-Latin script when it has any, else Latin; `None` when no letter belongs to a
/// known script. Latin stands for names and code in any language, so "запусти cargo test" is Cyrillic speech. Known
/// limit: English speech with one word in the target script ("send it to Сбербанк", into Russian) is then speech in the
/// target script, and only its length is checked.
fn speech_script(letters: &[char]) -> Option<Script> {
    let scripts = || letters.iter().filter_map(|c| script_of(*c));
    let mut counts: Vec<(Script, usize)> = Vec::new();
    for s in scripts().filter(|s| *s != Script::Latin) {
        match counts.iter_mut().find(|(x, _)| *x == s) {
            Some((_, n)) => *n += 1,
            None => counts.push((s, 1)),
        }
    }
    counts.into_iter().max_by_key(|(_, n)| *n).map(|(s, _)| s).or_else(|| scripts().find(|s| *s == Script::Latin))
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

    /// A case of the check: speech, result, dictionary, target language, verdict.
    type Case = (&'static str, &'static str, &'static [&'static str], &'static str, bool);
    const PASS: bool = true;
    const FAIL: bool = false;

    /// The cases whose verdict differs from the expected one.
    fn wrong_verdicts(cases: &[Case]) -> Vec<String> {
        cases
            .iter()
            .filter_map(|&(raw, out, terms, to, ok)| {
                let terms: Vec<String> = terms.iter().map(|t| t.to_string()).collect();
                (translation_ok(raw, out, &terms, to) != ok).then(|| format!("expected {ok} for {to}: {raw:?} → {out:?} {terms:?}"))
            })
            .collect()
    }

    #[test]
    fn translation_verdicts() {
        let cases: &[Case] = &[
            // Translations, with names and code kept as written in Latin.
            ("напомни мне завтра позвонить Ане в десять", "Remind me to call Anya at ten tomorrow.", &[], "en", PASS),
            ("remind me to call Anya tomorrow at ten", "Напомни мне завтра в десять позвонить Ане.", &[], "ru", PASS),
            ("открой Soniox и проверь ключ", "Open Soniox and check the key.", &[], "en", PASS),
            ("open Slack", "Открой Slack.", &[], "ru", PASS),
            ("run cargo test", "Запусти cargo test", &[], "ru", PASS),
            ("запусти cargo test", "Run cargo test.", &[], "en", PASS),
            ("check the GitHub pull request for CUE", "检查CUE的GitHub拉取请求", &[], "zh", PASS),
            ("run npm install then npm run build and npm test", "Запусти npm install, затем npm run build и npm test", &[], "ru", PASS),
            ("run cargo fmt then cargo clippy and cargo test", "Запусти cargo fmt, затем cargo clippy и cargo test.", &[], "ru", PASS),
            ("run git pull then git rebase main and git push force", "Выполни git pull, затем git rebase main и git push force.", &[], "ru", PASS),
            ("запусти пожалуйста npm install а потом npm run build", "Please run npm install and then npm run build.", &[], "en", PASS),
            ("first npm install then npm run build and npm test", "先npm install，再npm run build和npm test", &[], "zh", PASS),
            ("run npm install, npm run build and npm test", "npm install、npm run build、npm testを実行", &[], "ja", PASS),
            ("after npm install run npm run build and npm test", "npm install 후 npm run build와 npm test 실행", &[], "ko", PASS),
            // Mostly code: each Latin letter weighs as one, not as Han, so the result is not "three times longer".
            ("run npm install lodash express typescript", "npm install lodash express typescriptを実行", &[], "ja", PASS),
            // Latin code inside Chinese speech weighs as Latin.
            ("请帮我在终端里运行cargo clippy命令吧", "Please run cargo clippy in the terminal.", &[], "en", PASS),
            ("deploy this to cuber netis", "Разверни это в Kubernetes.", &["Kubernetes"], "ru", PASS),
            ("deploy this to cuber netis", "Разверни это в Kubernetes.", &[], "ru", PASS),
            ("短い", "Short.", &[], "en", PASS),
            (
                "我明天早上十点给你打电话然后我们讨论这个计划的细节",
                "I'll call you tomorrow at ten in the morning and we'll go over the details of this plan.",
                &[],
                "en",
                PASS,
            ),
            // Kana is a syllable, not a word: a Japanese translation is not "three times longer".
            ("install the new update now", "今すぐ新しいアップデートをインストールしてください", &[], "ja", PASS),
            // Number words of the speech written as digits keep the length.
            ("The budget is fifty, sorry, sixty thousand dollars.", "Бюджет — 60 000 долларов.", &[], "ru", PASS),
            ("the meeting is at three thirty on the twenty second", "Встреча в 15:30 22-го.", &[], "ru", PASS),
            // A number alone: nothing untranslated can be pasted, and the length is checked however short the speech.
            ("сто двадцать пять", "125", &[], "en", PASS),
            ("напомни мне позвонить маме в пять часов вечера", "5", &[], "en", FAIL),
            // One script: the length only.
            ("call me when you are home", "Ruf mich an, wenn du zu Hause bist.", &[], "de", PASS),
            // Cleanups instead of translations, with names respelled in Latin.
            ("открой гитхаб и проверь пул реквест", "Открой GitHub и проверь пул реквест.", &[], "en", FAIL),
            ("открой гитхаб и проверь пул реквест", "Открой GitHub и проверь pull request.", &[], "en", FAIL),
            ("открой гитхаб и проверь пул реквест", "Открой GitHub и проверь pull request.", &["GitHub", "pull request"], "en", FAIL),
            ("проверь пул реквест в гитхабе и открой фигму", "Проверь pull request в GitHub и открой Figma.", &[], "en", FAIL),
            ("запусти докер компоуз апп", "Запусти Docker Compose апп.", &[], "en", FAIL),
            ("открой сониокс и проверь ключ", "Открой Soniox и проверь ключ.", &[], "en", FAIL),
            ("открой сониокс и проверь ключ", "Открой Soniox и проверь ключ.", &["Soniox"], "en", FAIL),
            ("запусти cargo test", "Запусти cargo test.", &[], "en", FAIL),
            ("pay the bill in sberbank", "Pay the bill in Сбербанк.", &["Сбербанк"], "ru", FAIL),
            // A listed name is no evidence of translation.
            ("кубернетес и докер", "Kubernetes и Docker.", &["Kubernetes", "Docker"], "en", FAIL),
            // The original with its translation, echoes, the wrong language, nothing.
            ("напомни мне завтра позвонить маме", "Напомни мне завтра позвонить маме. Remind me to call mom tomorrow.", &[], "en", FAIL),
            ("напомни мне завтра позвонить Ане", "Напомни мне завтра позвонить Ане.", &[], "en", FAIL),
            ("call Anya tomorrow at ten please", "Call Anya tomorrow at ten, please.", &[], "ru", FAIL),
            ("открой Slack", "Открой Slack.", &[], "en", FAIL),
            // Echoes of code-heavy speech: the code kept from the speech shows no translation.
            ("запусти npm install lodash express typescript", "Запусти npm install lodash express typescript.", &[], "en", FAIL),
            (
                "сделай git commit amend no edit и git push force with lease",
                "Сделай git commit amend no edit и git push force with lease.",
                &[],
                "en",
                FAIL,
            ),
            ("open Slack and check the messages", "打开Slack并检查消息", &[], "ru", FAIL),
            ("hello", "   ", &[], "ru", FAIL),
        ];
        let wrong = wrong_verdicts(cases);
        assert!(wrong.is_empty(), "{} wrong verdicts:\n{}", wrong.len(), wrong.join("\n"));
    }

    /// Known limits of the rule (see `written_in`), pinned as they are.
    #[test]
    fn known_limits() {
        // A translation made only of listed terms leaves nothing to show it.
        assert!(!translation_ok("Sberbank", "Сбербанк", &["Сбербанк".to_owned()], "ru"));
        // Into a non-Latin target Latin text shows nothing either way: the English original followed by its translation
        // passes, and so does English with one word in the target script.
        let both = "Remind me to call mom tomorrow. Напомни мне завтра позвонить маме.";
        assert!(translation_ok("remind me to call mom tomorrow", both, &[], "ru"));
        assert!(translation_ok("pay the bill in sberbank", "Pay the bill in Сбербанк.", &[], "ru"));
        // A phrase of names alone: without the dictionary the respelled names read as English.
        assert!(translation_ok("кубернетес и докер", "Kubernetes и Docker.", &[], "en"));
        // English speech with a word in the target script reads as a pair in one script: the length only, so the echo
        // passes.
        let echo = "Send the invoice to Сбербанк today, please.";
        assert!(translation_ok("send the invoice to Сбербанк today please", echo, &[], "ru"));
        // A digit weighs less than most spoken numbers take: a long number alone, in many words and few digits, still
        // falls under the length bound.
        assert!(!translation_ok("one thousand nine hundred ninety nine", "1999", &[], "ru"));
        // The check does not read meaning: a self-correction turned around passes.
        let inverted = "Send the report to Igor, I mean Oleg, by the end of the day.";
        assert!(translation_ok("Отправь отчёт Олегу, точнее, Игорю, до конца дня.", inverted, &[], "en"));
    }

    /// Japanese writes kana and kanji, Korean Hangul syllables and jamo, full- or halfwidth.
    #[test]
    fn japanese_and_korean_letters_are_in_their_languages_scripts() {
        for c in "ひらがなカタカナー漢字ㇰｶﾀｶﾅ".chars() {
            assert_eq!(script_of(c), script_of_language("ja"), "{c} U+{:04X}", c as u32);
        }
        for c in "한국어ᄀㄱㅏﾡ".chars() {
            assert_eq!(script_of(c), script_of_language("ko"), "{c} U+{:04X}", c as u32);
        }
    }

    #[test]
    fn an_answer_instead_of_a_translation_fails_on_length() {
        let raw = "какая сегодня погода в Москве";
        let answer = "I can't check the weather, but you can open a weather site or app and search for Moscow to see the current forecast and temperature for today.";
        assert!(!translation_ok(raw, answer, &[], "en"));
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

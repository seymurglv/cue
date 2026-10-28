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

/// Letters of the final tokens per language: the speech language is the one with the most letters (spec 2 §5 — "by the
/// language of most words"; tokens are pieces of words, letters weigh them fairly). Tokens without a known language are
/// skipped.
#[derive(Debug, Default, Clone)]
pub struct LanguageTally {
    letters: Vec<(&'static str, usize)>,
}

impl LanguageTally {
    pub fn add(&mut self, language: &str, text: &str) {
        let Some(code) = known(language) else { return };
        let n = text.chars().filter(|c| c.is_alphabetic()).count();
        if n == 0 {
            return;
        }
        match self.letters.iter_mut().find(|(c, _)| *c == code) {
            Some((_, count)) => *count += n,
            None => self.letters.push((code, n)),
        }
    }

    /// The language with the most letters; on a tie, the one heard first.
    pub fn dominant(&self) -> Option<&'static str> {
        self.letters
            .iter()
            .fold(None, |best: Option<(&'static str, usize)>, &(c, n)| match best {
                Some((_, m)) if m >= n => best,
                _ => Some((c, n)),
            })
            .map(|(c, _)| c)
    }
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
    fn tally_picks_the_language_with_most_letters() {
        let mut t = LanguageTally::default();
        t.add("ru", "Скажи");
        t.add("en", " Soniox");
        t.add("ru", " что всё готово");
        assert_eq!(t.dominant(), Some("ru"));
        let mut t = LanguageTally::default();
        t.add("en", "ok");
        t.add("ru", "ок");
        assert_eq!(t.dominant(), Some("en"), "a tie goes to the language heard first");
        t.add("xx", "aaaaaaaaaa");
        assert_eq!(t.dominant(), Some("en"), "unknown codes are skipped");
        assert_eq!(LanguageTally::default().dominant(), None);
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
}

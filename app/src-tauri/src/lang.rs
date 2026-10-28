//! Interface language, decided once at startup: `ui.language` from settings.json when it is `en` or `ru`, otherwise
//! the Windows UI language (Russian when its primary language is Russian, English otherwise). The tray menu and the
//! Drop window both use this one value, so they always match. The same Windows language, as a locale name, is the
//! default home language of translation.
use cue_core::settings::UiLanguage;
use windows::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};

/// `LANG_RUSSIAN` from winnt.h.
const LANG_RUSSIAN: u16 = 0x19;
/// Global the Drop window reads before any of its scripts run (`pickLang` in `app/ui/src/strings.ts`).
const WINDOW_GLOBAL: &str = "__CUE_LANG__";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Ru,
}

impl Lang {
    /// The language for this run: a fixed `en` or `ru` from the settings wins; `auto` asks `windows` (`Lang::detect`).
    pub fn resolve(setting: UiLanguage, windows: impl FnOnce() -> Self) -> Self {
        match setting {
            UiLanguage::En => Self::En,
            UiLanguage::Ru => Self::Ru,
            UiLanguage::Auto => windows(),
        }
    }

    /// The user's Windows UI language.
    pub fn detect() -> Self {
        // SAFETY: no arguments; the call only reads the user's language setting.
        Self::from_langid(unsafe { GetUserDefaultUILanguage() })
    }

    /// Only the primary language counts (`PRIMARYLANGID`, the low 10 bits): ru-RU, ru-MD and the rest are all Russian.
    pub fn from_langid(langid: u16) -> Self {
        if langid & 0x3ff == LANG_RUSSIAN { Self::Ru } else { Self::En }
    }

    /// Code the Drop window understands.
    pub fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Ru => "ru",
        }
    }

    /// Initialization script for the Drop window: sets the language before the page's own scripts run.
    /// The code goes in through Rust's `{:?}` (a quoted Rust string literal). That is a valid JavaScript string only
    /// because the code is always the fixed ASCII `"en"` or `"ru"`; never pass other text into a script this way.
    pub fn init_script(self) -> String {
        format!("window.{WINDOW_GLOBAL} = {:?};", self.code())
    }
}

/// The Windows display language as a locale name ("ru-RU"), for the home language of translation (spec 2 §9).
/// `None` if Windows does not give it.
pub fn windows_language_code() -> Option<String> {
    // SAFETY: no arguments; reads the user's language setting.
    let langid = unsafe { GetUserDefaultUILanguage() };
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    // SAFETY: the buffer is writable and its length is passed; the LCID is SORT_DEFAULT with the LANGID.
    let n = unsafe { LCIDToLocaleName(u32::from(langid), Some(&mut buf), 0) };
    // `n` counts the terminating null; 0 is a failure.
    (n > 1).then(|| String::from_utf16_lossy(&buf[..n as usize - 1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russian_ui_language_gives_russian() {
        assert_eq!(Lang::from_langid(0x0419), Lang::Ru); // ru-RU
        assert_eq!(Lang::from_langid(0x0819), Lang::Ru); // ru-MD
        assert_eq!(Lang::from_langid(0x0019), Lang::Ru); // neutral Russian
    }

    #[test]
    fn any_other_ui_language_gives_english() {
        assert_eq!(Lang::from_langid(0x0409), Lang::En); // en-US
        assert_eq!(Lang::from_langid(0x0809), Lang::En); // en-GB
        assert_eq!(Lang::from_langid(0x0422), Lang::En); // uk-UA
        assert_eq!(Lang::from_langid(0x0423), Lang::En); // be-BY
        assert_eq!(Lang::from_langid(0x042C), Lang::En); // az-Latn-AZ
        assert_eq!(Lang::from_langid(0x0407), Lang::En); // de-DE
        assert_eq!(Lang::from_langid(0), Lang::En);
        // 0x19 in the sublanguage bits does not make it Russian.
        assert_eq!(Lang::from_langid((0x19 << 10) | 0x09), Lang::En);
    }

    #[test]
    fn settings_en_or_ru_win_over_windows() {
        let windows_not_asked = || -> Lang { panic!("Windows must not be asked when the language is set") };
        assert_eq!(Lang::resolve(UiLanguage::En, windows_not_asked), Lang::En);
        assert_eq!(Lang::resolve(UiLanguage::Ru, windows_not_asked), Lang::Ru);
    }

    #[test]
    fn settings_auto_follows_windows() {
        assert_eq!(Lang::resolve(UiLanguage::Auto, || Lang::from_langid(0x0419)), Lang::Ru);
        assert_eq!(Lang::resolve(UiLanguage::Auto, || Lang::from_langid(0x0409)), Lang::En);
    }

    /// The whole path from the file: an unknown value in settings.json means Windows decides.
    #[test]
    fn language_from_a_settings_file() {
        let pick = |json: &str, windows: Lang| {
            let s: cue_core::settings::Settings = serde_json::from_str(json).unwrap();
            Lang::resolve(s.ui.language, || windows)
        };
        assert_eq!(pick(r#"{"ui":{"language":"ru"}}"#, Lang::En), Lang::Ru);
        assert_eq!(pick(r#"{"ui":{"language":"en"}}"#, Lang::Ru), Lang::En);
        assert_eq!(pick(r#"{"ui":{"language":"auto"}}"#, Lang::Ru), Lang::Ru);
        assert_eq!(pick(r#"{"ui":{"language":"de"}}"#, Lang::Ru), Lang::Ru);
        assert_eq!(pick(r#"{}"#, Lang::En), Lang::En);
    }

    /// Reads this machine's setting: any Windows has a display language, and its locale name comes back without the
    /// terminating null.
    #[test]
    fn windows_gives_its_language_as_a_locale_name() {
        let code = windows_language_code().expect("Windows has a display language");
        assert!(!code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{code:?}");
    }

    #[test]
    fn window_gets_the_code() {
        assert_eq!(Lang::En.code(), "en");
        assert_eq!(Lang::Ru.code(), "ru");
        assert_eq!(Lang::Ru.init_script(), r#"window.__CUE_LANG__ = "ru";"#);
        assert_eq!(Lang::En.init_script(), r#"window.__CUE_LANG__ = "en";"#);
    }
}

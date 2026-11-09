//! The cleanup's instructions (spec "no keys", Instructions page): CUE's style rules in plain words, kept in one file,
//! `%APPDATA%\CUE\instructions\instructions.md`, that the user or an AI edits. The folder holds only the instructions'
//! files, so an AI opened in it sees nothing else of CUE's. The fixed protocol (output format, never answering
//! the text, the dictionary and habits blocks, the translate instruction) stays in code and is never in the file.
//! The standard texts, the `AGENTS.md` CUE writes beside the file and the task handed to an AI live here; reading and
//! writing the file is the app's (`instructions` in the app crate).
use crate::refine::{BOLD_PROMPT, BOLD_PROMPT_RU};
use crate::settings::MAX_CUSTOM_CHARS;

/// The standard instructions for the interface language (`"ru"`, anything else is English): today's cleanup.
pub fn standard(lang: &str) -> &'static str {
    if lang == "ru" { BOLD_PROMPT_RU } else { BOLD_PROMPT }
}

/// The instructions a request may use: trimmed, cut at [`MAX_CUSTOM_CHARS`] characters; `None` when nothing is left.
pub fn usable(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let end = text.char_indices().nth(MAX_CUSTOM_CHARS).map_or(text.len(), |(i, _)| i);
    Some(text[..end].trim_end())
}

/// The task for an AI that edits the instructions (given as the first message of an agent, or copied for any AI).
pub fn task(lang: &str) -> String {
    if lang == "ru" {
        "Нужно поправить инструкции CUE для чистки диктовок. Правь только файл instructions.md в этой папке. \
Сначала прочитай AGENTS.md. Спроси меня, что изменить, и поправь только то, о чём я прошу. \
Когда сохранишь, попроси меня открыть в CUE страницу «Ваши инструкции»: она показывает, как было и как стало на моей \
последней диктовке; это и есть проверка. Не трогай ключи, входы в аккаунты и другие настройки CUE."
            .to_owned()
    } else {
        "I want to change CUE's instructions for cleaning up dictations. Edit only instructions.md in this folder. \
Read AGENTS.md first. Ask me what to change and change only what I ask for. \
After you save, ask me to open CUE's Instructions page: it shows before and after on my last dictation, and that is \
the check. Do not touch keys, sign-ins or any other CUE setting."
            .to_owned()
    }
}

/// The task copied for any AI ("Скопировать задание"): [`task`], then the folder it is about, since that AI may be opened
/// anywhere.
pub fn task_with_folder(lang: &str, folder: &str) -> String {
    let label = if lang == "ru" { "Папка" } else { "Folder" };
    format!("{}\n\n{label}: {folder}", task(lang))
}

/// `AGENTS.md`, which CUE owns and rewrites when its version is newer. Its first line, `<!-- CUE <version> -->`, is what
/// decides "older" ([`agents_md_version`]).
pub fn agents_md(lang: &str, version: &str) -> String {
    let body = if lang == "ru" {
        "\
# Инструкции CUE

CUE чистит диктовки: убирает слова-паразиты, исправляет речь и ставит знаки. Файл `instructions.md` в этой папке — \
правила стиля этой чистки простыми словами. CUE читает файл при следующей диктовке после сохранения.

## Что можно менять

Только `instructions.md`: правила стиля, простым текстом, не длиннее 2000 символов (остальное CUE не использует). \
Без подстановок и разметки. Формат ответа, запрет отвечать на продиктованный текст, словарь, привычки и перевод \
заданы в самом CUE и в файле не нужны.

## Что нельзя менять

Ничего вне этой папки: настройки CUE, Диспетчер учётных данных Windows (ключи и входы), историю диктовок. И этот \
`AGENTS.md`: CUE перепишет его сам.

## Как проверить

Спросите пользователя, что изменить. После сохранения попросите его открыть в CUE страницу «Ваши инструкции»: она \
показывает, как было и как стало на его последней диктовке. Предыдущая версия лежит рядом в `instructions.prev.md`.
"
    } else {
        "\
# CUE instructions

CUE cleans up dictations: it removes filler words, fixes the speech and sets punctuation. The file `instructions.md` \
in this folder holds the style rules of that cleanup in plain words. CUE reads the file at the next dictation after \
it is saved.

## What you may change

Only `instructions.md`: style rules in plain text, at most 2000 characters (CUE uses no more). No placeholders and no \
markup. The answer format, never answering the dictated text, the dictionary, the habits and translation are set in \
CUE itself and are not needed in the file.

## What you may not change

Nothing outside this folder: CUE's settings, the Windows Credential Manager (keys and sign-ins), the dictation \
history. Nor this `AGENTS.md`, which CUE rewrites by itself.

## How to check

Ask the user what to change. After saving, ask them to open CUE's Instructions page: it shows before and after on \
their last dictation. The previous version is kept beside the file as `instructions.prev.md`.
"
    };
    format!("<!-- CUE {version} -->\n\n{body}")
}

/// The version in the first line of an `AGENTS.md` CUE wrote (`<!-- CUE <version> -->`); `None` for any other file.
pub fn agents_md_version(text: &str) -> Option<&str> {
    let line = text.lines().next()?.trim();
    line.strip_prefix("<!-- CUE ")?.strip_suffix(" -->").map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_texts_exist_in_both_languages() {
        assert_eq!(standard("en"), BOLD_PROMPT);
        assert_eq!(standard("ru"), BOLD_PROMPT_RU);
        assert_ne!(standard("en"), standard("ru"));
        assert!(standard("ru").contains("Смелая правка") && standard("en").contains("Bold editing"));
    }

    #[test]
    fn usable_trims_and_caps_at_2000() {
        assert_eq!(usable("  пиши коротко \n"), Some("пиши коротко"));
        let long = "я".repeat(MAX_CUSTOM_CHARS + 500);
        assert_eq!(usable(&long).unwrap().chars().count(), MAX_CUSTOM_CHARS);
        assert_eq!(usable(&"x".repeat(MAX_CUSTOM_CHARS)).unwrap().chars().count(), MAX_CUSTOM_CHARS);
    }

    #[test]
    fn usable_rejects_blank() {
        assert_eq!(usable(""), None);
        assert_eq!(usable(" \n\t "), None);
    }

    #[test]
    fn task_names_the_file_agents_md_and_the_page() {
        let en = task("en");
        for needed in ["instructions.md", "AGENTS.md", "Instructions page", "keys"] {
            assert!(en.contains(needed), "{needed}");
        }
        let ru = task("ru");
        for needed in ["instructions.md", "AGENTS.md", "Ваши инструкции", "ключи"] {
            assert!(ru.contains(needed), "{needed}");
        }
        assert!(!en.contains("cue-cli") && !ru.contains("cue-cli"));
    }

    /// The copied task goes to an AI opened anywhere: it names the folder, and asks for the page, never a CLI.
    #[test]
    fn copied_task_names_the_folder() {
        let folder = r"C:\Users\Ann\AppData\Roaming\CUE\instructions";
        for lang in ["ru", "en"] {
            let copied = task_with_folder(lang, folder);
            assert!(copied.starts_with(&task(lang)), "{lang}");
            assert!(copied.ends_with(folder), "{lang}");
            assert!(!copied.contains("cue-cli"), "{lang}");
        }
        assert!(task_with_folder("ru", folder).contains("Папка: "));
        assert!(task_with_folder("en", folder).contains("Folder: "));
    }

    /// The task is one line with no double quote or percent sign: it goes to an agent as its first message on a
    /// command line (`terminal`).
    #[test]
    fn task_is_one_line_without_quotes() {
        for lang in ["ru", "en"] {
            let t = task(lang);
            assert!(!t.contains(['\n', '\r', '"', '%']), "{lang}");
        }
    }

    /// AGENTS.md says the folder holds only the instructions, and that nothing outside it is the agent's.
    #[test]
    fn agents_md_keeps_the_agent_in_the_folder() {
        assert!(agents_md("ru", "1").contains("вне этой папки"));
        assert!(agents_md("en", "1").contains("outside this folder"));
    }

    #[test]
    fn agents_md_starts_with_its_version() {
        let text = agents_md("en", "0.9.1");
        assert!(text.starts_with("<!-- CUE 0.9.1 -->\n"));
        assert_eq!(agents_md_version(&text), Some("0.9.1"));
        assert_eq!(agents_md_version(&agents_md("ru", "1.0.0")), Some("1.0.0"));
        assert_eq!(agents_md_version("# my own file"), None);
        assert!(agents_md("en", "1").contains("instructions.md") && agents_md("ru", "1").contains("instructions.md"));
    }
}

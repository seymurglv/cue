//! What every cleanup route shares (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): the system text CUE gives a
//! model — its prompt, the dictionary, the habits and the job's own block — the `{"text": …}` answer read from what
//! the model wrote, and the checks that answer passes before it is pasted ([`settle`]). The user's agent and the local
//! model both use them. Never "fails": on any problem the raw text comes back, with one log line of the reason.
use cue_core::flow::RefineOutcome;
use cue_core::refine::{habits_prompt, sanity_ok_with, strip_markup, terms_prompt, tidy_list_items, SYSTEM_PROMPT};
use cue_core::settings::{DictionarySettings, RefineSettings};
use cue_core::translate::{self, translation_ok};
use serde_json::Value;

/// What the cleanup does with the transcript.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// Clean up in the spoken language (dictation).
    Clean,
    /// Clean up and turn spoken phrasing into written: CUE's own cleanup of a dictation
    /// ([`cue_core::refine::BOLD_PROMPT`]) or the user's instructions file in its place.
    Rewrite,
    /// Clean up and write in this language (a code of `cue_core::translate::LANGUAGES`).
    Translate(&'static str),
}

/// The blocks of the system text, in order: CUE's prompt; the dictionary block ([`terms_prompt`]); for a cleanup, the
/// habits block ([`habits_prompt`]); then the job's own block: CUE's cleanup (the `instructions`, the instructions
/// file's text as given) or the translation's. The prompt and the dictionary come first: they change least.
pub(crate) fn system_texts(dict: &DictionarySettings, habits: &[String], job: Job, instructions: &str) -> Vec<String> {
    let mut system = vec![SYSTEM_PROMPT.to_owned()];
    system.extend(terms_prompt(&dict.terms, &dict.learned));
    if matches!(job, Job::Clean | Job::Rewrite) {
        system.extend(habits_prompt(habits));
    }
    match job {
        Job::Clean => {}
        Job::Rewrite => system.push(instructions.to_owned()),
        Job::Translate(to) => system.push(translate::instruction(to)),
    }
    system
}

/// The system blocks ([`system_texts`]) joined into one system text: what an agent and the local model read.
pub(crate) fn system_text(dict: &DictionarySettings, habits: &[String], job: Job, instructions: &str) -> String {
    system_texts(dict, habits, job, instructions).join("\n\n")
}

/// The tokens a request reported (usage.csv counts them). The user's agent and the local model report none.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
}

/// What the model's answer was.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    Text(String),
    Refusal,
    Truncated,
    Malformed,
}

/// The model's answer → parse result: cut by length is `Truncated`, a filter is `Refusal`; otherwise the `text` field
/// of a JSON object (also inside a ```` ``` ```` fence). Without a schema (`structured == false`) the whole plain text is
/// accepted too, without a fence around it — except an empty one and a JSON fragment.
pub fn parse_content(content: &str, finish_reason: Option<&str>, structured: bool) -> Parsed {
    match finish_reason {
        Some("length") => return Parsed::Truncated,
        Some("content_filter") => return Parsed::Refusal,
        _ => {}
    }
    let content = content.trim();
    let json = unfence(content);
    if let Some(text) = serde_json::from_str::<Value>(json).ok().and_then(|v| v.get("text")?.as_str().map(str::to_owned)) {
        return Parsed::Text(text);
    }
    if structured || json.is_empty() || json.starts_with('{') {
        return Parsed::Malformed;
    }
    Parsed::Text(json.to_owned())
}

/// JSON inside a ```` ```json … ``` ```` fence → what is inside; otherwise as is.
fn unfence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else { return text };
    let Some((_, body)) = rest.split_once('\n') else { return text };
    body.trim_end().strip_suffix("```").map(str::trim).unwrap_or(text)
}

/// The refined text if it passes the sanity check; otherwise the raw one.
/// Markdown markup is stripped before the check ([`strip_markup`]), and so are trailing commas and semicolons
/// of list items ([`tidy_list_items`]): the check judges what will be pasted. Words of the dictionary terms count as
/// said ([`sanity_ok_with`]).
pub fn decide(raw: &str, parsed: Parsed, dict: &DictionarySettings, s: &RefineSettings) -> (String, RefineOutcome) {
    decide_with(raw, parsed, dict, &[], s)
}

/// [`decide`] for a request that sent habits: the new words of their replacements (`said`, `RefineContext::said`)
/// count as said too ([`sanity_ok_with`]), for this answer only.
pub fn decide_with(raw: &str, parsed: Parsed, dict: &DictionarySettings, said: &[String], s: &RefineSettings) -> (String, RefineOutcome) {
    match parsed {
        Parsed::Text(t) => {
            let t = tidy_list_items(&strip_markup(raw, &t));
            if sanity_ok_with(raw, &t, &dict.terms, said, s) {
                (t.trim().to_string(), RefineOutcome::Refined)
            } else {
                (raw.to_string(), RefineOutcome::RawFallback)
            }
        }
        _ => (raw.to_string(), RefineOutcome::RawFallback),
    }
}

/// The translation if it passes [`translation_ok`]; markup and list punctuation are stripped first, as in [`decide`].
pub fn decide_translation(raw: &str, parsed: Parsed, dict: &DictionarySettings, to: &str) -> (String, RefineOutcome) {
    match parsed {
        Parsed::Text(t) => {
            let t = tidy_list_items(&strip_markup(raw, &t));
            if translation_ok(raw, &t, &dict.terms, to) {
                (t.trim().to_string(), RefineOutcome::Refined)
            } else {
                (raw.to_string(), RefineOutcome::RawFallback)
            }
        }
        _ => (raw.to_string(), RefineOutcome::RawFallback),
    }
}

/// Refine result: what to paste, why exactly this, and how many tokens were spent (for usage accounting).
pub struct RefineResult {
    pub text: String,
    pub outcome: RefineOutcome,
    pub usage: Usage,
}

/// A parsed answer → what to paste: the answer if it passes its job's check, otherwise the raw text and one log line
/// with the reason. `said` are the words the habits sent with a cleanup write ([`decide_with`]); a translation's check
/// takes none.
pub(crate) fn settle(
    raw: &str,
    parsed: Parsed,
    dict: &DictionarySettings,
    said: &[String],
    s: &RefineSettings,
    job: Job,
    usage: Usage,
) -> RefineResult {
    // The reason, in case the answer is rejected.
    let why = match (&parsed, job) {
        (Parsed::Text(_), Job::Clean | Job::Rewrite) => "response failed the sanity check",
        (Parsed::Text(_), Job::Translate(_)) => "translation failed the check",
        (Parsed::Refusal, _) => "model refused",
        (Parsed::Truncated, _) => "response truncated at max_tokens",
        (Parsed::Malformed, _) => "response does not match the schema",
    };
    let decided = match job {
        Job::Clean | Job::Rewrite => decide_with(raw, parsed, dict, said, s),
        Job::Translate(to) => decide_translation(raw, parsed, dict, to),
    };
    match decided {
        (text, RefineOutcome::Refined) => RefineResult { text, outcome: RefineOutcome::Refined, usage },
        (_, outcome) => raw_fallback(raw, outcome, usage, why),
    }
}

/// The raw text instead of the refined one, plus one log line: the reason and the outcome.
pub(crate) fn raw_fallback(raw: &str, outcome: RefineOutcome, usage: Usage, why: &str) -> RefineResult {
    tracing::warn!("refine: {why} → {outcome:?}");
    RefineResult { text: raw.to_string(), outcome, usage }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::refine::{sanity_ok, BOLD_PROMPT};

    fn dict(terms: &[&str]) -> DictionarySettings {
        DictionarySettings { terms: terms.iter().map(|t| t.to_string()).collect(), ..Default::default() }
    }

    /// Habits go after the dictionary, for a cleanup only; with none the text is the same.
    #[test]
    fn habits_go_after_the_dictionary_for_a_cleanup_only() {
        let rules = vec!["В Telegram — с маленькой буквы и без точки".to_owned()];
        let block = habits_prompt(&rules).unwrap();
        let texts = system_texts(&dict(&["Soniox"]), &rules, Job::Rewrite, BOLD_PROMPT);
        assert_eq!((texts.len(), texts[0].as_str(), texts[2].as_str(), texts[3].as_str()), (4, SYSTEM_PROMPT, block.as_str(), BOLD_PROMPT));
        assert_eq!(system_texts(&dict(&[]), &rules, Job::Clean, BOLD_PROMPT), [SYSTEM_PROMPT.to_owned(), block]);
        let to_en = system_text(&dict(&[]), &rules, Job::Translate("en"), BOLD_PROMPT);
        assert!(!to_en.contains("без точки"), "a translation takes none");
        assert_eq!(system_texts(&dict(&[]), &[], Job::Clean, BOLD_PROMPT), [SYSTEM_PROMPT]);
    }

    /// With no usable term there is no dictionary block.
    #[test]
    fn without_terms_there_is_no_dictionary_block() {
        let long = "x".repeat(61);
        for terms in [dict(&[]), dict(&["", "  ", "Claude\nHaiku", &long])] {
            assert_eq!(system_texts(&terms, &[], Job::Clean, BOLD_PROMPT), [SYSTEM_PROMPT], "{:?}", terms.terms);
        }
    }

    #[test]
    fn a_learned_word_reaches_the_system_text_with_how_it_sounds() {
        let dict = DictionarySettings {
            terms: vec!["Marisol".to_owned()],
            learned: vec![cue_core::settings::LearnedWord { word: "Marisol".into(), heard: "Марисоль".into() }],
        };
        let texts = system_texts(&dict, &[], Job::Clean, BOLD_PROMPT);
        assert!(texts[1].contains("Marisol — sounds like «Марисоль»"), "{}", texts[1]);
    }

    #[test]
    fn terms_go_in_a_second_block_after_the_prompt() {
        let block = "Names and terms the user often says. When the speech clearly refers to one of them, even if the transcript \
                     spells it differently, splits it into parts or mixes alphabets, write it exactly as listed. Don't add a term \
                     the user didn't say, and don't change other words because of this list.\n\nSoniox\nGitHub";
        assert_eq!(system_texts(&dict(&["Soniox", " GitHub "]), &[], Job::Clean, BOLD_PROMPT), [SYSTEM_PROMPT, block]);
        assert_eq!(system_text(&dict(&["Soniox", " GitHub "]), &[], Job::Clean, BOLD_PROMPT), format!("{SYSTEM_PROMPT}\n\n{block}"));
    }

    #[test]
    fn a_translation_adds_its_block_last() {
        for t in [dict(&[]), dict(&["Soniox"])] {
            let clean = system_texts(&t, &[], Job::Clean, BOLD_PROMPT);
            let tr = system_texts(&t, &[], Job::Translate("en"), BOLD_PROMPT);
            assert_eq!(tr.len(), clean.len() + 1);
            assert_eq!(tr[..clean.len()], clean[..], "the blocks before it are the cleanup's");
            assert_eq!(tr.last().unwrap(), &translate::instruction("en"));
            assert!(tr.last().unwrap().contains("English"));
        }
        assert!(system_texts(&dict(&["Soniox"]), &[], Job::Translate("ru"), BOLD_PROMPT)[2].contains("Russian"));
    }

    /// The instructions file's text is the cleanup's block as given; a translation is the same whatever it says.
    #[test]
    fn the_instructions_are_the_cleanup_block_as_given() {
        for t in [dict(&[]), dict(&["Soniox"])] {
            let mut bold = system_texts(&t, &[], Job::Clean, BOLD_PROMPT);
            bold.push(BOLD_PROMPT.to_owned());
            assert_eq!(system_texts(&t, &[], Job::Rewrite, BOLD_PROMPT), bold);
            *bold.last_mut().unwrap() = "пиши коротко".to_owned();
            assert_eq!(system_texts(&t, &[], Job::Rewrite, "пиши коротко"), bold);
            let to_en = Job::Translate("en");
            assert_eq!(system_texts(&t, &[], to_en, "пиши коротко"), system_texts(&t, &[], to_en, BOLD_PROMPT));
        }
    }

    #[test]
    fn parse_content_reads_text_field() {
        assert_eq!(parse_content(r#"{"text":"Привет."}"#, Some("stop"), true), Parsed::Text("Привет.".into()));
        assert_eq!(parse_content(" \n{\"text\": \"Привет.\"}\n", Some("stop"), false), Parsed::Text("Привет.".into()));
        assert_eq!(parse_content("```json\n{\"text\":\"Привет.\"}\n```", Some("stop"), true), Parsed::Text("Привет.".into()));
        assert_eq!(parse_content(r#"{"text":"Привет."}"#, None, true), Parsed::Text("Привет.".into()), "no finish reason, the text is still usable");
    }

    #[test]
    fn parse_content_plain_text_fallback_only_without_schema() {
        assert_eq!(parse_content("  Привет, как дела?\n", Some("stop"), false), Parsed::Text("Привет, как дела?".into()));
        assert_eq!(parse_content("Привет, как дела?", Some("stop"), true), Parsed::Malformed, "the schema was requested, so plain text is not accepted");
        // Plain text in a fence: taken without the fence; an empty fence is nothing.
        assert_eq!(parse_content("```\nПривет, как дела?\n```", None, false), Parsed::Text("Привет, как дела?".into()));
        assert_eq!(parse_content("```text\nПривет, как дела?\n```\n", None, false), Parsed::Text("Привет, как дела?".into()));
        assert_eq!(parse_content("```\n```", None, false), Parsed::Malformed);
        assert_eq!(parse_content("```\nПривет, как дела?\n```", None, true), Parsed::Malformed);
        for broken in [r#"{"text":"Привет"#, r#"{"answer":"Привет"}"#, "", "  \n"] {
            assert_eq!(parse_content(broken, Some("stop"), false), Parsed::Malformed, "{broken:?}");
            assert_eq!(parse_content(broken, Some("stop"), true), Parsed::Malformed, "{broken:?}");
        }
    }

    #[test]
    fn parse_content_truncated_and_filtered() {
        assert_eq!(parse_content(r#"{"text":"Прив"#, Some("length"), true), Parsed::Truncated);
        assert_eq!(parse_content(r#"{"text":"Привет."}"#, Some("length"), false), Parsed::Truncated, "cut off, even if the JSON is intact");
        assert_eq!(parse_content("", Some("content_filter"), true), Parsed::Refusal);
    }

    #[test]
    fn decide_uses_raw_when_insane_or_failed() {
        let s = RefineSettings::default();
        assert_eq!(decide("ну привет как дела", Parsed::Text("Привет, как дела?".into()), &Default::default(), &s), ("Привет, как дела?".into(), RefineOutcome::Refined));
        let raw = "напиши письмо ане про встречу в четверг";
        let (t, o) = decide(raw, Parsed::Text("Конечно! Вот письмо для Ани: Дорогая Аня, напоминаю о встрече в четверг. С уважением.".into()), &Default::default(), &s);
        assert_eq!((t.as_str(), o), (raw, RefineOutcome::RawFallback));
        assert_eq!(decide(raw, Parsed::Refusal, &Default::default(), &s).1, RefineOutcome::RawFallback);
    }

    #[test]
    fn decide_keeps_a_dictionary_fix_in_a_short_phrase() {
        let s = RefineSettings::default();
        let fixed = || Parsed::Text("Открой GitHub.".into());
        assert_eq!(decide("открой гитхаб", fixed(), &dict(&["GitHub"]), &s), ("Открой GitHub.".into(), RefineOutcome::Refined));
        assert_eq!(decide("открой гитхаб", fixed(), &Default::default(), &s).1, RefineOutcome::RawFallback, "without the dictionary: 1 of 2 words said");
    }

    /// A replacement habit sent with the cleanup writes words that were never said; its answer is kept, and the words
    /// count for this answer only: a translation's check is the same with them or without.
    #[test]
    fn settle_keeps_the_words_of_a_habit_sent_with_the_cleanup() {
        let s = RefineSettings::default();
        let (raw, greeting) = ("здравствуйте анна", || Parsed::Text("Добрый день, Анна!".into()));
        let said = vec!["добрый день".to_owned()];
        let no = DictionarySettings::default();
        let with = settle(raw, greeting(), &no, &said, &s, Job::Rewrite, Usage::default());
        assert_eq!((with.text.as_str(), with.outcome), ("Добрый день, Анна!", RefineOutcome::Refined));
        let without = settle(raw, greeting(), &no, &[], &s, Job::Rewrite, Usage::default());
        assert_eq!((without.text.as_str(), without.outcome), (raw, RefineOutcome::RawFallback));
        assert_eq!(settle(raw, greeting(), &no, &said, &s, Job::Clean, Usage::default()).outcome, RefineOutcome::Refined);
        for answer in ["Hello, Anna!", "Добрый день, Анна!"] {
            let translated = |said: &[String]| {
                let r = settle(raw, Parsed::Text(answer.into()), &no, said, &s, Job::Translate("en"), Usage::default());
                (r.text, r.outcome)
            };
            assert_eq!(translated(&said), translated(&[]), "{answer}");
        }
        assert_eq!(no, DictionarySettings::default(), "nothing went into the dictionary");
    }

    #[test]
    fn decide_strips_markdown_before_the_sanity_check() {
        let s = RefineSettings::default();
        // The heading and bold are stripped: plain text is pasted.
        let raw = "ну итоги встречи релиз в четверг и бюджет согласован";
        let answer = "# Итоги встречи\n**Релиз** в четверг, бюджет согласован.";
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &Default::default(), &s),
            ("Итоги встречи\nРелиз в четверг, бюджет согласован.".into(), RefineOutcome::Refined)
        );
        // The check judges what will be pasted: with the asterisks the response is over 25 % longer than what was said.
        let raw = "ну купить молоко хлеб сыр масло";
        let answer = "Купить:\n* **молоко**\n* **хлеб**\n* **сыр**\n* **масло**";
        assert!(!sanity_ok(raw, answer, &[], &s), "with the markup the check fails");
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &Default::default(), &s),
            ("Купить:\n- молоко\n- хлеб\n- сыр\n- масло".into(), RefineOutcome::Refined)
        );
    }

    #[test]
    fn decide_drops_trailing_separators_of_list_items() {
        let s = RefineSettings::default();
        // A trailing ";" of list items is stripped before the check; the last item's period and lowercase letters stay.
        let raw = "взять с собой зонт куртку и термос";
        let answer = "Взять с собой:\n- зонт;\n- куртку;\n- термос.";
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &Default::default(), &s),
            ("Взять с собой:\n- зонт\n- куртку\n- термос.".into(), RefineOutcome::Refined)
        );
        // Together with markup stripping: `* ` → `- `, then the trailing commas.
        let answer = "Взять с собой:\n* **зонт**,\n* **куртку**,\n* **термос**";
        assert_eq!(
            decide(raw, Parsed::Text(answer.into()), &Default::default(), &s),
            ("Взять с собой:\n- зонт\n- куртку\n- термос".into(), RefineOutcome::Refined)
        );
    }

    #[test]
    fn decide_translation_checks_the_script_not_the_words() {
        let raw = "ну привет";
        assert_eq!(decide_translation(raw, Parsed::Text("**Hello.**".into()), &Default::default(), "en"), ("Hello.".into(), RefineOutcome::Refined));
        assert_eq!(decide_translation(raw, Parsed::Text("Ну привет.".into()), &Default::default(), "en"), (raw.into(), RefineOutcome::RawFallback));
        assert_eq!(decide_translation(raw, Parsed::Truncated, &Default::default(), "en"), (raw.into(), RefineOutcome::RawFallback));
    }

    /// A translation is checked against the dictionary too: an answer that is the dictionary's terms written over the
    /// speech, the rest left in its language, is not a translation, and the raw text comes back.
    #[test]
    fn translate_checks_the_answer_against_the_dictionary_too() {
        let raw = "кубернетес и докер";
        let (t, o) = decide_translation(raw, Parsed::Text("Kubernetes и Docker.".into()), &dict(&["Kubernetes", "Docker"]), "en");
        assert_eq!((t.as_str(), o), (raw, RefineOutcome::RawFallback));
    }
}

//! The cleanup, a translation and a habit's rule on this computer (docs/specs/2026-11-04-local-cleanup.md): the same
//! system text and user message a chat service gets ([`chat::system_text`], `user_message`), answered by the local
//! model ([`Generate`]) under a grammar, and the same checks after ([`claude::settle`]). Never "fails": on any problem
//! the raw text comes back; nothing leaves the computer and nothing costs.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use cue_core::flow::RefineOutcome;
use cue_core::habits::{self, RuleAsk, RuleText};
use cue_core::refine::{self, RefineContext, user_message};
use cue_core::settings::{DictionarySettings, RefineSettings};
use cue_core::translate;

use crate::EngineError;
use crate::chat;
use crate::claude::{self, Job, Parsed, RefineResult, Usage};
use crate::local_llm::{Generate, RULE_GRAMMAR, TEXT_GRAMMAR};
use crate::openrouter::parse_content;

/// The time a piece of a dictation gets on this computer, whatever its length: a processor is slow and nobody has
/// measured it.
pub const LOCAL_HEADROOM_MS: u64 = 20_000;
/// And for each of its words: a model on a processor writes a minute of speech in about 20 s on a fast desktop, two or
/// three times slower on a laptop; this is twice that.
pub const LOCAL_MS_PER_WORD: u64 = 400;
/// A long dictation is cleaned in pieces of about this many words, one after another, each with the text before it
/// (the caret's and the pieces already cleaned) as its context: a piece fits the model's context and has its own
/// deadline, and a dictation of any length is cleaned.
pub const PIECE_WORDS: usize = 80;

/// How long a piece of `raw` may take on this computer.
pub fn local_timeout_ms(raw: &str) -> u64 {
    LOCAL_HEADROOM_MS + LOCAL_MS_PER_WORD * raw.split_whitespace().count() as u64
}

/// The longest a warm-up evaluates before giving up; what it evaluated stays.
const WARM_FOR: Duration = Duration::from_secs(60);

/// Sets its flag when dropped: a cleanup whose task was cancelled (or that ended) stops the model at its next step,
/// so the model is free for the next dictation.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The model's answer, asked on a blocking thread; dropping this future stops the model.
async fn answer(llm: Arc<dyn Generate>, system: String, user: String, grammar: &'static str, deadline: Instant) -> Option<Result<String, EngineError>> {
    let stop = Arc::new(AtomicBool::new(false));
    let _stop = StopOnDrop(stop.clone());
    tokio::task::spawn_blocking(move || llm.generate(&system, &user, grammar, deadline, &stop)).await.ok()
}

/// Evaluates the cleanup's system part for a dictation just started, with the habits it will send, while the person
/// speaks: its cleanup then pays only for its own words. In the background; a cleanup waits for it and goes on from
/// what it evaluated.
pub fn warm(llm: Arc<dyn Generate>, dict: &DictionarySettings, habits: &[String], s: &RefineSettings) {
    let system = chat::system_text(dict, habits, Job::Rewrite, s.custom_text());
    let deadline = Instant::now() + WARM_FOR;
    tokio::task::spawn_blocking(move || llm.warm(&system, deadline, &AtomicBool::new(false)));
}

/// The raw text at once while no local model is ready (not chosen, downloading, loading): dictation never waits.
pub fn not_ready(raw: &str) -> RefineResult {
    claude::raw_fallback(raw, RefineOutcome::RawFallback, Usage::default(), "local model not ready")
}

/// One cleanup on this computer, in the style of `s`.
pub async fn refine(llm: Arc<dyn Generate>, raw: &str, ctx: &RefineContext, dict: &DictionarySettings, s: &RefineSettings) -> RefineResult {
    run(llm, raw, ctx, dict, s, Job::Rewrite).await
}

/// One translation into `to` on this computer.
pub async fn translate(
    llm: Arc<dyn Generate>,
    raw: &str,
    ctx: &RefineContext,
    dict: &DictionarySettings,
    s: &RefineSettings,
    to: &'static str,
) -> RefineResult {
    run(llm, raw, ctx, dict, s, Job::Translate(to)).await
}

/// A dictation, a piece at a time ([`refine::pieces`]). A piece the model could not clean stays as said; once one runs
/// out of time, the rest stays as said too, since the next would take as long. A translation is whole or not at all:
/// a piece left in the language spoken fails it. A translation gets no text before the caret: a model on a processor
/// writes it, translated, into its answer, where it cannot be found and cut.
async fn run(llm: Arc<dyn Generate>, raw: &str, ctx: &RefineContext, dict: &DictionarySettings, s: &RefineSettings, job: Job) -> RefineResult {
    let translating = matches!(job, Job::Translate(_));
    let plain;
    let ctx = if translating {
        plain = RefineContext { before_cursor: None, ..ctx.clone() };
        &plain
    } else {
        ctx
    };
    let parts = refine::pieces(raw, PIECE_WORDS);
    if parts.len() < 2 {
        return one(llm, raw, ctx, dict, s, job).await.0;
    }
    let before = ctx.before_cursor.clone().unwrap_or_default();
    let (mut text, mut cleaned, mut late) = (String::new(), 0usize, false);
    for (i, part) in parts.iter().enumerate() {
        let piece = part.trim();
        let mut pctx = RefineContext::new(ctx.app.clone(), (!translating).then(|| format!("{before}{text}")).as_deref(), CONTEXT_CHARS);
        (pctx.habits, pctx.said) = (ctx.habits.clone(), ctx.said.clone());
        let r = if late || piece.is_empty() {
            RefineResult { text: piece.to_owned(), outcome: RefineOutcome::RawFallback, usage: Usage::default() }
        } else {
            let (r, timed_out) = one(llm.clone(), piece, &pctx, dict, s, job).await;
            late = timed_out;
            r
        };
        if r.outcome == RefineOutcome::Refined {
            cleaned += 1;
        } else if translating {
            return claude::raw_fallback(raw, RefineOutcome::RawFallback, Usage::default(), "local model: a piece was not translated");
        }
        text.push_str(&r.text);
        if i + 1 < parts.len() {
            // As the pieces were parted: on a new line, or a space.
            text.push_str(if part[part.trim_end().len()..].contains('\n') { "\n" } else { " " });
        }
    }
    tracing::info!(pieces = parts.len(), cleaned, "local model: a long dictation in pieces");
    let outcome = if cleaned > 0 { RefineOutcome::Refined } else { RefineOutcome::RawFallback };
    RefineResult { text, outcome, usage: Usage::default() }
}

/// The context a piece of a long dictation gets of the text before it, as the cleanup's own (`RefineContext::new`).
const CONTEXT_CHARS: usize = 500;

/// One request to the local model; `true` with it when it ran out of time.
async fn one(llm: Arc<dyn Generate>, raw: &str, ctx: &RefineContext, dict: &DictionarySettings, s: &RefineSettings, job: Job) -> (RefineResult, bool) {
    let extra = match job {
        Job::Clean | Job::Rewrite => 0,
        Job::Translate(_) => translate::timeout_ms(raw) - refine::timeout_ms(raw),
    };
    let deadline = Instant::now() + Duration::from_millis(local_timeout_ms(raw) + extra);
    let system = chat::system_text(dict, &ctx.habits, job, s.custom_text());
    let user = user_message(ctx, raw);
    match answer(llm, system, user, TEXT_GRAMMAR, deadline).await {
        Some(Ok(content)) => {
            let parsed = match parse_content(&content, None, true) {
                Parsed::Text(t) => Parsed::Text(refine::drop_copied_context(&t, ctx.before_cursor.as_deref(), raw).to_owned()),
                other => other,
            };
            (claude::settle(raw, parsed, dict, &ctx.said, s, job, Usage::default()), false)
        }
        Some(Err(e)) => (claude::raw_fallback(raw, RefineOutcome::RawFallback, Usage::default(), &format!("local model: {}", why(&e))), e == EngineError::Timeout),
        None => (claude::raw_fallback(raw, RefineOutcome::RawFallback, Usage::default(), "local model: stopped"), false),
    }
}

/// The reason of a local failure for the log: its kind, never a text.
fn why(e: &EngineError) -> &'static str {
    match e {
        EngineError::Timeout => "timeout",
        EngineError::Llm(_) => "cannot run",
        _ => "failed",
    }
}

/// A habit's rule from the local model, within `timeout`; none when it fails or the answer is not a usable rule.
pub async fn rule(llm: Arc<dyn Generate>, _s: &RefineSettings, ask: &RuleAsk, lang: &str, timeout: Duration) -> Option<RuleText> {
    let (system, user) = habits::rule_messages(ask, lang);
    let deadline = Instant::now() + timeout;
    let content = match answer(llm, system, user, RULE_GRAMMAR, deadline).await? {
        Ok(content) => content,
        Err(e) => {
            tracing::info!("habit rule: local model {}, written from the template", why(&e));
            return None;
        }
    };
    let rule = habits::read_rule(&content);
    if rule.is_none() {
        tracing::info!("habit rule: the local answer is not a usable rule, written from the template");
    }
    rule
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use cue_core::connections::find;
    use cue_core::habits::{Draft, Form};
    use cue_core::refine::user_message;
    use cue_core::settings::RefineStyle;

    /// Stands for the model: records what it was asked and answers as it was told.
    struct Fake {
        answer: Result<String, EngineError>,
        asked: Mutex<Vec<(String, String, String, Instant)>>,
    }

    impl Fake {
        fn new(answer: Result<&str, EngineError>) -> Arc<Self> {
            Arc::new(Self { answer: answer.map(str::to_owned), asked: Mutex::new(Vec::new()) })
        }
        fn asked(&self) -> (String, String, String, Instant) {
            self.asked.lock().unwrap()[0].clone()
        }
    }

    impl Generate for Fake {
        fn generate(&self, system: &str, user: &str, grammar: &str, deadline: Instant, _: &AtomicBool) -> Result<String, EngineError> {
            self.asked.lock().unwrap().push((system.into(), user.into(), grammar.into(), deadline));
            self.answer.clone()
        }
        fn id(&self) -> &str {
            "llm--qwen--qwen3-4b-gguf--q4_k_m"
        }
    }

    const RAW: &str = "ну привет как дела";

    fn ctx() -> RefineContext {
        RefineContext::new("Telegram", Some("Вчера договорились. "), 500)
    }

    #[tokio::test]
    async fn local_refine_sends_the_cloud_prompt() {
        let s = RefineSettings { style: RefineStyle::Custom, custom: "пиши коротко".into(), ..RefineSettings::default() };
        let dict = DictionarySettings { terms: vec!["GitHub".into()], ..Default::default() };
        let with = RefineContext { habits: vec!["No full stop".into()], ..ctx() };
        let fake = Fake::new(Ok(r#"{"text":"Ну, привет. Как дела?"}"#));
        let r = refine(fake.clone(), RAW, &with, &dict, &s).await;
        assert_eq!(r.outcome, RefineOutcome::Refined);
        let (system, user, grammar, _) = fake.asked();
        let cloud = crate::chat::request_body(find("openai").unwrap(), "gpt-6-luna", RAW, &with, &dict, Job::Rewrite, Some("пиши коротко"));
        assert_eq!(system, cloud["messages"][0]["content"].as_str().unwrap(), "the same rules, dictionary, habits and style");
        assert_eq!(user, user_message(&with, RAW));
        assert_eq!(grammar, TEXT_GRAMMAR);
    }

    #[tokio::test]
    async fn local_refine_checks_the_answer() {
        let s = RefineSettings::default();
        let r = refine(Fake::new(Ok(r#"{"text":"Ну, привет. Как дела?"}"#)), RAW, &ctx(), &Default::default(), &s).await;
        assert_eq!((r.text.as_str(), r.outcome), ("Ну, привет. Как дела?", RefineOutcome::Refined));
        // A reply instead of the text: the raw text.
        let r = refine(Fake::new(Ok(r#"{"text":"Конечно! Ну, привет. Как дела?"}"#)), RAW, &ctx(), &Default::default(), &s).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
        // Broken JSON: the raw text.
        let r = refine(Fake::new(Ok(r#"{"text":"Ну, при"#)), RAW, &ctx(), &Default::default(), &s).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
        // The model failed: the raw text.
        let r = refine(Fake::new(Err(EngineError::Llm("does not load".into()))), RAW, &ctx(), &Default::default(), &s).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
    }

    /// A small model's copy of the text before the cursor is not pasted twice.
    #[tokio::test]
    async fn local_refine_drops_a_copied_context() {
        let r = refine(Fake::new(Ok(r#"{"text":"Вчера договорились. Ну, привет. Как дела?"}"#)), RAW, &ctx(), &Default::default(), &RefineSettings::default()).await;
        assert_eq!((r.text.as_str(), r.outcome), ("Ну, привет. Как дела?", RefineOutcome::Refined));
    }

    #[tokio::test]
    async fn local_deadline_grows_with_the_words() {
        assert_eq!(local_timeout_ms(""), LOCAL_HEADROOM_MS);
        assert_eq!(local_timeout_ms(&"слово ".repeat(80)), LOCAL_HEADROOM_MS + 80 * LOCAL_MS_PER_WORD);
    }

    /// Answers each piece with its own words, and runs out of time on the call `late` (from 0).
    struct Echo {
        late: Option<usize>,
        asked: Mutex<Vec<String>>,
    }

    impl Generate for Echo {
        fn generate(&self, _: &str, user: &str, _: &str, _: Instant, _: &AtomicBool) -> Result<String, EngineError> {
            let mut asked = self.asked.lock().unwrap();
            asked.push(user.to_owned());
            if self.late == Some(asked.len() - 1) {
                return Err(EngineError::Timeout);
            }
            let said = user.split("<transcript>").nth(1).and_then(|t| t.split("</transcript>").next()).unwrap();
            Ok(serde_json::json!({ "text": said }).to_string())
        }
        fn id(&self) -> &str {
            "echo"
        }
    }

    /// Three pieces: sentences of 50, 50 and 40 words, the last on a new line.
    fn long_dictation() -> String {
        let sentence = |n: usize| format!("{}конец.", "слово ".repeat(n - 1));
        format!("{} {}\n{}", sentence(50), sentence(50), sentence(40))
    }

    /// The text before the caret a request sent.
    fn before_cursor(user: &str) -> &str {
        user.split("<before_cursor>").nth(1).and_then(|t| t.split("</before_cursor>").next()).unwrap_or("")
    }

    #[tokio::test]
    async fn a_long_dictation_is_cleaned_a_piece_at_a_time() {
        let raw = long_dictation();
        let echo = Arc::new(Echo { late: None, asked: Mutex::new(Vec::new()) });
        let r = refine(echo.clone(), &raw, &ctx(), &Default::default(), &RefineSettings::default()).await;
        assert_eq!(r.outcome, RefineOutcome::Refined);
        assert_eq!(r.text, raw, "the pieces are put back as they were parted");
        let asked = echo.asked.lock().unwrap();
        assert_eq!(asked.len(), 3);
        assert!(!before_cursor(&asked[0]).contains("конец."), "the first piece has only the caret's context");
        assert!(before_cursor(&asked[2]).contains("конец.\n"), "a later piece sees the text cleaned before it");
    }

    #[tokio::test]
    async fn after_a_piece_runs_out_of_time_the_rest_stays_as_said() {
        let raw = long_dictation();
        let echo = Arc::new(Echo { late: Some(1), asked: Mutex::new(Vec::new()) });
        let r = refine(echo.clone(), &raw, &ctx(), &Default::default(), &RefineSettings::default()).await;
        assert_eq!(r.outcome, RefineOutcome::Refined, "the first piece was cleaned");
        assert_eq!(r.text, raw);
        assert_eq!(echo.asked.lock().unwrap().len(), 2, "the third piece was not asked");
    }

    #[tokio::test]
    async fn a_translation_with_a_piece_left_is_not_pasted() {
        let raw = long_dictation();
        let echo = Arc::new(Echo { late: Some(2), asked: Mutex::new(Vec::new()) });
        let r = translate(echo, &raw, &ctx(), &Default::default(), &RefineSettings::default(), "en").await;
        assert_eq!((r.text.as_str(), r.outcome), (raw.as_str(), RefineOutcome::RawFallback));
    }

    #[tokio::test]
    async fn local_refine_stops_at_the_deadline() {
        let fake = Fake::new(Err(EngineError::Timeout));
        let before = Instant::now();
        let r = refine(fake.clone(), RAW, &ctx(), &Default::default(), &RefineSettings::default()).await;
        assert_eq!((r.text.as_str(), r.outcome), (RAW, RefineOutcome::RawFallback));
        let (.., deadline) = fake.asked();
        let expected = before + Duration::from_millis(local_timeout_ms(RAW));
        let off = if deadline > expected { deadline - expected } else { expected - deadline };
        assert!(off < Duration::from_millis(100), "{off:?}");
    }

    #[test]
    fn not_ready_is_the_raw_text_at_once() {
        let r = not_ready(RAW);
        assert_eq!((r.text.as_str(), r.outcome, r.usage), (RAW, RefineOutcome::RawFallback, Usage::default()));
    }

    #[tokio::test]
    async fn local_refine_costs_nothing() {
        let r = refine(Fake::new(Ok(r#"{"text":"Ну, привет. Как дела?"}"#)), RAW, &ctx(), &Default::default(), &RefineSettings::default()).await;
        assert_eq!(r.usage, Usage::default());
    }

    #[tokio::test]
    async fn local_translate_uses_the_translation_check() {
        let fake = Fake::new(Ok(r#"{"text":"Well, hi. How are you?"}"#));
        let r = translate(fake.clone(), RAW, &ctx(), &Default::default(), &RefineSettings::default(), "en").await;
        assert_eq!((r.text.as_str(), r.outcome), ("Well, hi. How are you?", RefineOutcome::Refined));
        let (system, user, ..) = fake.asked();
        assert!(system.contains(&cue_core::translate::instruction("en")));
        assert!(!user.contains("<before_cursor>"), "a small model would write it, translated, into its answer");
        // A "translation" still in the speaker's alphabet fails the check.
        let r = translate(Fake::new(Ok(r#"{"text":"Ну, привет. Как дела?"}"#)), RAW, &ctx(), &Default::default(), &RefineSettings::default(), "en").await;
        assert_eq!(r.outcome, RefineOutcome::RawFallback);
    }

    /// CUE's own cleanup, a translation and a habit's rule on a real model, `CUE_TEST_GGUF=<file>`: each answer is
    /// printed with its outcome and time, for a person to read; the test fails only when a job falls back to the raw
    /// text. Synthetic dictations only.
    #[tokio::test]
    #[ignore = "needs a GGUF model file"]
    async fn local_jobs_on_a_real_model() {
        let file = std::env::var("CUE_TEST_GGUF").expect("CUE_TEST_GGUF");
        /// Prints the model's answer as generated, before CUE's checks.
        struct Shown(Arc<crate::local_llm::LocalLlm>);
        impl Generate for Shown {
            fn generate(&self, system: &str, user: &str, grammar: &str, deadline: Instant, stop: &AtomicBool) -> Result<String, EngineError> {
                let answer = self.0.generate(system, user, grammar, deadline, stop);
                eprintln!("  answer: {answer:?}");
                answer
            }
            fn id(&self) -> &str {
                self.0.id()
            }
        }
        let llm: Arc<dyn Generate> = Arc::new(Shown(crate::local_llm::LocalLlm::load(std::path::Path::new(&file), "smoke").unwrap()));
        let s = RefineSettings::default();
        let dict = DictionarySettings { terms: vec!["GitHub".into()], ..Default::default() };
        let mut fell_back = Vec::new();
        let english = RefineContext::new("Slack", Some("Sounds good. "), 500);
        for (raw, ctx) in [
            ("ну короче я посмотрел пулреквест на гитхабе там всё нормально можно мержить", ctx()),
            ("so um I think we should move the meeting to thursday because uh friday is a holiday", english),
        ] {
            let started = Instant::now();
            let r = refine(llm.clone(), raw, &ctx, &dict, &s).await;
            eprintln!("refine {:?} -> {:?} ({:?}, {} ms)", raw, r.text, r.outcome, started.elapsed().as_millis());
            if r.outcome != RefineOutcome::Refined {
                fell_back.push(raw);
            }
        }
        let raw = "привет давай созвонимся завтра в три";
        let started = Instant::now();
        let r = translate(llm.clone(), raw, &ctx(), &dict, &s, "en").await;
        eprintln!("translate {:?} -> {:?} ({:?}, {} ms)", raw, r.text, r.outcome, started.elapsed().as_millis());
        if r.outcome != RefineOutcome::Refined {
            fell_back.push(raw);
        }
        let ask = Draft { forms: vec![Form::FirstLetter { lower: true }], app: "Telegram".into(), before: "Скинь файл.".into(), after: "скинь файл".into(), n: 3 }.ask();
        let started = Instant::now();
        let rule = rule(llm, &s, &ask, "en", Duration::from_secs(30)).await;
        eprintln!("rule -> {rule:?} ({} ms)", started.elapsed().as_millis());
        assert!(fell_back.is_empty(), "fell back to the raw text: {fell_back:?}");
        assert!(rule.is_some(), "no habit rule");
    }

    #[tokio::test]
    async fn local_rule_reads_the_rule_or_none() {
        let ask = Draft { forms: vec![Form::FirstLetter { lower: true }], app: "Telegram".into(), before: "Скинь файл.".into(), after: "скинь файл".into(), n: 3 }.ask();
        let fake = Fake::new(Ok(r#"{"rule":"In Telegram — lower case","short":"lower case"}"#));
        let r = rule(fake.clone(), &RefineSettings::default(), &ask, "en", Duration::from_secs(5)).await.unwrap();
        assert_eq!((r.rule.as_str(), r.short.as_str()), ("In Telegram — lower case", "lower case"));
        let (system, user, grammar, _) = fake.asked();
        assert_eq!((system, user), cue_core::habits::rule_messages(&ask, "en"));
        assert_eq!(grammar, RULE_GRAMMAR);
        assert!(rule(Fake::new(Ok(r#"{"rule":"","short":""}"#)), &RefineSettings::default(), &ask, "en", Duration::from_secs(5)).await.is_none());
        assert!(rule(Fake::new(Err(EngineError::Timeout)), &RefineSettings::default(), &ask, "en", Duration::from_secs(5)).await.is_none());
    }
}

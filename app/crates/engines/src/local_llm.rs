//! The cleanup's language model on this computer (docs/specs/2026-11-04-local-cleanup.md): a GGUF model run by
//! llama.cpp on the processor, behind [`Generate`] so the requests (`local`) and the runtime can be tested without one.
//! The part of a prompt already evaluated in the model's context is kept for the next one ([`common_prefix`]); the
//! answer is held to the `{"text": …}` object by a grammar ([`TEXT_GRAMMAR`]).
use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::token::data::LlamaTokenData;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;

use crate::EngineError;

/// A language model that answers a system and a user message, by `deadline`, in the shape `grammar` allows
/// ([`TEXT_GRAMMAR`] for a cleanup or a translation, [`RULE_GRAMMAR`] for a habit's rule).
pub trait Generate: Send + Sync {
    /// The answer as generated (the JSON object), or why there is none: `Timeout` past the deadline or once `stop` is
    /// set (the cleanup was cancelled), `Llm` for a model that cannot run.
    fn generate(&self, system: &str, user: &str, grammar: &str, deadline: Instant, stop: &AtomicBool) -> Result<String, EngineError>;
    /// Evaluates the part of a prompt that `system` makes, ahead of a dictation, so its cleanup pays only for its own
    /// words. Gives up at `deadline` or once `stop` is set, keeping what it evaluated. Nothing for a model that keeps
    /// nothing.
    fn warm(&self, _system: &str, _deadline: Instant, _stop: &AtomicBool) {}
    /// The model's id (its folder under `models`).
    fn id(&self) -> &str;
}

/// A model in a log line or a `Debug`: its id only.
impl std::fmt::Debug for dyn Generate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Generate").field(&self.id()).finish()
    }
}

/// GBNF for exactly one JSON object with one string field, `text`.
pub const TEXT_GRAMMAR: &str = r#"root ::= "{" ws "\"text\"" ws ":" ws string ws "}"
string ::= "\"" char* "\""
char ::= [^"\\\x7F\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F])
ws ::= [ \t\n]{0,20}
"#;

/// GBNF for a habit's rule: one JSON object with the string fields `rule` and `short`.
pub const RULE_GRAMMAR: &str = r#"root ::= "{" ws "\"rule\"" ws ":" ws string ws "," ws "\"short\"" ws ":" ws string ws "}"
string ::= "\"" char* "\""
char ::= [^"\\\x7F\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F])
ws ::= [ \t\n]{0,20}
"#;

/// GBNF for an "Ask" answer on this computer: `answer`, `write` and `replace`, and `sources` always empty, since the
/// model here never searches.
pub const ASK_GRAMMAR: &str = r#"root ::= "{" ws "\"answer\"" ws ":" ws string ws "," ws "\"write\"" ws ":" ws bool ws "," ws "\"replace\"" ws ":" ws bool ws "," ws "\"sources\"" ws ":" ws "[" ws "]" ws "}"
bool ::= "true" | "false"
string ::= "\"" char* "\""
char ::= [^"\\\x7F\x00-\x1F] | "\\" (["\\/bfnrt] | "u" [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F] [0-9a-fA-F])
ws ::= [ \t\n]{0,20}
"#;

/// How many tokens `next` shares from its start with `kept`, the tokens already evaluated in the context: those stay,
/// the rest is evaluated again. The system part of a prompt (CUE's rules, the dictionary, the habits) comes first, so a
/// dictation pays only for its own words while the system part stays the same, and for all of it once it changes.
pub fn common_prefix<T: PartialEq>(kept: &[T], next: &[T]) -> usize {
    kept.iter().zip(next).take_while(|(a, b)| a == b).count()
}

/// Whether a model's chat template writes Gemma 4's turns (`<|turn>role … <turn|>`), a format llama.cpp's built-in
/// templates do not know: CUE writes those itself ([`turn_prompt`]).
pub fn uses_turns(template: &str) -> bool {
    template.contains("<|turn>")
}

/// A system and a user message in Gemma 4's turns, as its template writes them with thinking off: each content
/// trimmed, then the model's turn opened. The tokenizer adds the BOS.
pub fn turn_prompt(system: &str, user: &str) -> String {
    format!("<|turn>system\n{}<turn|>\n<|turn>user\n{}<turn|>\n<|turn>model\n", system.trim(), user.trim())
}

/// Threads for generation, from the efficiency class of each physical core (higher is faster). On a processor with
/// fast and efficient cores (Intel's P and E), every fast core: llama.cpp's threads wait for the slowest at each step,
/// so an efficient core would hold the fast ones back, and the efficient cores keep the rest of the computer
/// responsive. On a processor with one kind, every core but one, for the same. One at least.
pub fn threads(cores: &[u8]) -> usize {
    let Some(&fast) = cores.iter().max() else { return 1 };
    let fast_cores = cores.iter().filter(|&&c| c == fast).count();
    if fast_cores < cores.len() { fast_cores } else { cores.len().saturating_sub(1).max(1) }
}

/// The context the model works in: a prompt and its answer fit in it.
const CONTEXT_TOKENS: u32 = 4096;
/// Tokens evaluated at once.
const BATCH_TOKENS: usize = 512;
/// The longest answer.
const MAX_ANSWER_TOKENS: usize = 1024;

/// llama.cpp's backend, set up once per process, its log silenced (it would print to the console).
fn backend() -> Result<&'static LlamaBackend, EngineError> {
    static BACKEND: OnceLock<Result<LlamaBackend, String>> = OnceLock::new();
    BACKEND
        .get_or_init(|| {
            LlamaBackend::init().map(|mut b| {
                b.void_logs();
                b
            })
            .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| EngineError::Llm(format!("backend: {e}")))
}

/// The efficiency class of each physical core of this computer, as Windows tells it; when it cannot, as many equal
/// cores as half the logical processors (two threads a core is the common case).
fn physical_cores() -> Vec<u8> {
    cores_from_windows().unwrap_or_else(|| vec![0; std::thread::available_parallelism().map_or(2, |n| n.get() / 2)])
}

#[cfg(windows)]
fn cores_from_windows() -> Option<Vec<u8>> {
    use windows::Win32::System::SystemInformation::{GetLogicalProcessorInformationEx, RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX};
    let mut len = 0u32;
    // SAFETY: with no buffer Windows only writes the length it needs.
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, None, &mut len) };
    if len == 0 {
        return None;
    }
    // u64 words: the records are 8-byte aligned.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    let first = buf.as_mut_ptr().cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>();
    // SAFETY: `buf` holds `len` bytes, as Windows asked.
    unsafe { GetLogicalProcessorInformationEx(RelationProcessorCore, Some(first), &mut len) }.ok()?;
    let mut cores = Vec::new();
    let mut at = 0usize;
    let base = buf.as_ptr().cast::<u8>();
    while at + std::mem::size_of::<u32>() * 2 <= len as usize {
        // SAFETY: `at` is the start of a record Windows wrote within `len`; each says its own size.
        let record = unsafe { &*base.add(at).cast::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>() };
        if record.Size == 0 {
            break;
        }
        if record.Relationship == RelationProcessorCore {
            // SAFETY: a core's record holds the processor relationship.
            cores.push(unsafe { record.Anonymous.Processor.EfficiencyClass });
        }
        at += record.Size as usize;
    }
    (!cores.is_empty()).then_some(cores)
}

#[cfg(not(windows))]
fn cores_from_windows() -> Option<Vec<u8>> {
    None
}

/// The index of the highest logit, the first of equals: the greedy choice.
pub fn best(logits: &[f32]) -> usize {
    logits.iter().enumerate().fold(0, |b, (i, &l)| if l > logits[b] { i } else { b })
}

/// The next token, greedy under the grammar. The grammar is asked about the model's own choice first, which it nearly
/// always allows: checking it against the whole vocabulary (150 000 tokens and more) at every step would take most of
/// the time. Only when it refuses is the vocabulary filtered, and the best allowed token taken — the same token either
/// way.
fn pick(ctx: &LlamaContext<'_>, idx: i32, grammar: &mut LlamaSampler) -> LlamaToken {
    let logits = ctx.get_logits_ith(idx);
    let first = best(logits);
    let token = LlamaToken::new(i32::try_from(first).unwrap_or(0));
    let mut one = LlamaTokenDataArray::new(vec![LlamaTokenData::new(token, logits[first], 0.0)], false);
    grammar.apply(&mut one);
    if one.data.first().is_some_and(|d| d.logit() > f32::NEG_INFINITY) {
        return token;
    }
    let mut all = ctx.token_data_array_ith(idx);
    grammar.apply(&mut all);
    all.sample_token_greedy()
}

/// How much of `next` to keep of what the context holds (`kept`): their common start, less the last token when that is
/// all of `next`, since the last token is evaluated again for its logits.
pub fn reuse<T: PartialEq>(kept: &[T], next: &[T]) -> usize {
    let same = common_prefix(kept, next);
    if same == next.len() { same.saturating_sub(1) } else { same }
}

/// The model and its context. The context borrows the model, which lives at a fixed address from `Box::into_raw`
/// until `Drop` frees it, after the context.
struct Loaded {
    context: Option<LlamaContext<'static>>,
    /// The model's chat template for llama.cpp to apply; `None` for Gemma 4's turns ([`turn_prompt`]).
    template: Option<LlamaChatTemplate>,
    /// The tokens evaluated in the context now: the last prompt and its answer, or as much of a prompt as was evaluated
    /// before a deadline.
    kept: Vec<LlamaToken>,
    model: *mut LlamaModel,
}

// SAFETY: llama.cpp's model and context are used from one thread at a time, behind `LocalLlm`'s mutex.
unsafe impl Send for Loaded {}

impl Drop for Loaded {
    fn drop(&mut self) {
        self.context = None;
        // SAFETY: `model` came from `Box::into_raw` in `LocalLlm::load`, and nothing borrows it once the context is gone.
        drop(unsafe { Box::from_raw(self.model) });
    }
}

impl Loaded {
    fn model(&self) -> &'static LlamaModel {
        // SAFETY: `model` stays valid until `Drop`, which drops the context, its only other borrower, first.
        unsafe { &*self.model }
    }

    /// The prompt of a system and a user message, as tokens.
    fn tokens(&self, system: &str, user: &str) -> Result<Vec<LlamaToken>, EngineError> {
        let model = self.model();
        let prompt = match &self.template {
            Some(template) => {
                let chat = [
                    LlamaChatMessage::new("system".into(), system.into()).map_err(|_| EngineError::Llm("a NUL in the prompt".into()))?,
                    LlamaChatMessage::new("user".into(), user.into()).map_err(|_| EngineError::Llm("a NUL in the prompt".into()))?,
                ];
                model.apply_chat_template(template, &chat, true).map_err(|e| EngineError::Llm(format!("chat template: {e}")))?
            }
            None => turn_prompt(system, user),
        };
        let vocab = model.vocab();
        // A template may write the BOS itself; a vocabulary may have none (its id is -1, which llama.cpp cannot spell).
        let add_bos = vocab.should_add_bos() && vocab.bos().0 >= 0 && {
            let bos = String::from_utf8_lossy(&vocab.token_to_piece(vocab.bos(), true, None)).into_owned();
            bos.is_empty() || !prompt.starts_with(&bos)
        };
        Ok(vocab.tokenize(prompt.as_bytes(), add_bos, true))
    }

    /// Brings the context to hold `tokens`: what it holds of them stays, the rest is evaluated. Past `deadline` or once
    /// `stop` is set it gives up with `Timeout`, and what it evaluated stays for the next prompt.
    fn evaluate(&mut self, tokens: &[LlamaToken], batch: &mut LlamaBatch, deadline: Instant, stop: &AtomicBool) -> Result<(), EngineError> {
        let ctx = self.context.as_mut().ok_or_else(|| EngineError::Llm("no context".into()))?;
        let mut same = reuse(&self.kept, tokens);
        // A memory that cannot drop a part (a recurrent model's) is emptied whole.
        if !ctx.clear_kv_cache_seq(Some(0), u32::try_from(same).ok(), None).unwrap_or(false) {
            ctx.clear_kv_cache();
            same = 0;
        }
        self.kept.truncate(same);
        for (start, chunk) in tokens[same..].chunks(BATCH_TOKENS).enumerate() {
            if Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
                return Err(EngineError::Timeout);
            }
            batch.clear();
            let first = same + start * BATCH_TOKENS;
            for (k, &t) in chunk.iter().enumerate() {
                let pos = i32::try_from(first + k).unwrap_or(i32::MAX);
                let last = first + k == tokens.len() - 1;
                batch.add(t, pos, &[0], last).map_err(|e| EngineError::Llm(format!("batch: {e}")))?;
            }
            if let Err(e) = ctx.decode(batch) {
                // What the failed batch left in the context is unknown: nothing is kept.
                self.kept.clear();
                return Err(EngineError::Llm(format!("decode: {e}")));
            }
            self.kept.extend_from_slice(chunk);
        }
        Ok(())
    }
}

/// A GGUF model run by llama.cpp on the processor.
pub struct LocalLlm {
    id: String,
    loaded: Mutex<Loaded>,
}

impl LocalLlm {
    /// The model in `file`, with its context. A file llama.cpp cannot load, or one whose chat template it cannot
    /// write, is `Llm`: it does not run in CUE.
    pub fn load(file: &Path, id: &str) -> Result<Arc<LocalLlm>, EngineError> {
        let backend = backend()?;
        let model = LlamaModel::load_from_file(backend, file, &LlamaModelParams::default())
            .map_err(|e| EngineError::Llm(format!("not loaded: {e}")))?;
        // From here `Loaded` owns the model and frees it on every way out.
        let mut loaded = Loaded { context: None, template: None, kept: Vec::new(), model: Box::into_raw(Box::new(model)) };
        let model = loaded.model();
        let template = model.chat_template(None).map_err(|_| EngineError::Llm("no chat template".into()))?;
        loaded.template = if template.to_str().is_ok_and(uses_turns) { None } else { Some(template) };
        // A template llama.cpp does not know fails here, not at the first dictation.
        loaded.tokens("system", "user")?;
        let cores = physical_cores();
        let n = i32::try_from(threads(&cores)).unwrap_or(1);        tracing::info!(cores = cores.len(), threads = n, "language model: threads");
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(CONTEXT_TOKENS))
            .with_n_batch(BATCH_TOKENS as u32)
            .with_n_threads(n)
            .with_n_threads_batch(n);
        loaded.context = Some(model.new_context(backend, params).map_err(|e| EngineError::Llm(format!("no context: {e}")))?);
        Ok(Arc::new(LocalLlm { id: id.to_owned(), loaded: Mutex::new(loaded) }))
    }
}

impl Generate for LocalLlm {
    fn generate(&self, system: &str, user: &str, grammar: &str, deadline: Instant, stop: &AtomicBool) -> Result<String, EngineError> {
        let mut guard = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        let l = &mut *guard;
        let tokens = l.tokens(system, user)?;
        if tokens.len() + MAX_ANSWER_TOKENS > CONTEXT_TOKENS as usize {
            return Err(EngineError::Llm("the prompt does not fit".into()));
        }
        let mut batch = LlamaBatch::new(BATCH_TOKENS, 1);
        l.evaluate(&tokens, &mut batch, deadline, stop)?;
        let model = l.model();
        let ctx = l.context.as_mut().ok_or_else(|| EngineError::Llm("no context".into()))?;
        let vocab = model.vocab();
        let mut grammar = LlamaSampler::grammar(model, grammar, "root").map_err(|e| EngineError::Llm(format!("grammar: {e}")))?;
        let mut answer: Vec<u8> = Vec::new();
        let mut idx = batch.n_tokens() - 1;
        for pos in tokens.len()..tokens.len() + MAX_ANSWER_TOKENS {
            if Instant::now() >= deadline || stop.load(Ordering::Relaxed) {
                return Err(EngineError::Timeout);
            }
            let token = pick(ctx, idx, &mut grammar);
            grammar.accept(token);
            if vocab.is_eog(token) {
                break;
            }
            answer.extend(vocab.token_to_piece(token, false, None));
            batch.clear();
            batch.add(token, i32::try_from(pos).unwrap_or(i32::MAX), &[0], true).map_err(|e| EngineError::Llm(format!("batch: {e}")))?;
            if let Err(e) = ctx.decode(&mut batch) {
                l.kept.clear();
                return Err(EngineError::Llm(format!("decode: {e}")));
            }
            l.kept.push(token);
            idx = 0;
        }
        String::from_utf8(answer).map_err(|_| EngineError::Llm("the answer is not UTF-8".into()))
    }

    fn warm(&self, system: &str, deadline: Instant, stop: &AtomicBool) {
        let mut guard = self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        let l = &mut *guard;
        // The prompt with an empty user message: everything up to where a dictation's own words begin is the same.
        let Ok(tokens) = l.tokens(system, "") else { return };
        if tokens.len() + MAX_ANSWER_TOKENS > CONTEXT_TOKENS as usize {
            return;
        }
        let mut batch = LlamaBatch::new(BATCH_TOKENS, 1);
        let started = Instant::now();
        if l.evaluate(&tokens, &mut batch, deadline, stop).is_ok() {
            tracing::debug!(tokens = tokens.len(), ms = started.elapsed().as_millis() as u64, "local model warmed");
        }
    }

    fn id(&self) -> &str {
        &self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_leave_one_core() {
        assert_eq!(threads(&[0; 8]), 7);
        assert_eq!(threads(&[0; 2]), 1);
        assert_eq!(threads(&[0]), 1);
        assert_eq!(threads(&[]), 1);
    }

    #[test]
    fn best_is_the_first_highest_logit() {
        assert_eq!(best(&[0.1, 2.0, -1.0, 2.0]), 1);
        assert_eq!(best(&[f32::NEG_INFINITY, -5.0]), 1);
        assert_eq!(best(&[]), 0);
    }

    #[test]
    fn threads_take_the_fast_cores_of_a_hybrid_processor() {
        // An i7-12700: eight P-cores, four E-cores.
        let cores = [[1u8; 8].as_slice(), &[0; 4]].concat();
        assert_eq!(threads(&cores), 8);
        assert_eq!(threads(&[0, 0, 1, 1, 1, 1]), 4, "in any order");
        assert_eq!(threads(&[2, 1, 1]), 1, "the fastest class only");
    }

    #[test]
    fn the_same_start_is_kept() {
        let system = [1, 2, 3, 4];
        assert_eq!(common_prefix(&[1, 2, 3, 4, 9, 9], &[1, 2, 3, 4, 7]), 4, "the same system part, another dictation");
        assert_eq!(common_prefix(&system, &system), 4);
        assert_eq!(common_prefix::<i32>(&[], &[1, 2]), 0, "nothing evaluated yet");
    }

    /// What a warm-up or a timed-out prompt left in the context is reused; a prompt the context holds whole has its
    /// last token evaluated again, for the logits the answer starts from.
    #[test]
    fn reuse_keeps_what_was_evaluated() {
        assert_eq!(reuse(&[1, 2, 3], &[1, 2, 3, 4, 5]), 3, "a warm-up's system part, or a prompt cut by a deadline");
        assert_eq!(reuse(&[1, 2, 3, 4, 5], &[1, 2, 3]), 2, "all of it held: the last again");
        assert_eq!(reuse(&[1, 2, 3], &[1, 2, 3]), 2);
        assert_eq!(reuse::<i32>(&[], &[]), 0);
        assert_eq!(reuse(&[9], &[1, 2]), 0);
    }

    /// Review focus: a dictionary word or a habit changes the system part, and nothing after the change is reused.
    #[test]
    fn a_changed_system_part_is_evaluated_again() {
        assert_eq!(common_prefix(&[1, 2, 3, 4, 5], &[1, 2, 8, 4, 5]), 2);
    }

    /// Gemma 4's template (`<|turn>…<turn|>`) is not among llama.cpp's built-in formats: CUE writes its turns itself,
    /// as the template does with thinking off — trimmed contents, then the model's turn opened.
    #[test]
    fn gemma4_turns_are_written_as_its_template_does() {
        assert!(uses_turns("{{- '<|turn>system\\n' -}} … {{- '<turn|>\\n' -}}"));
        assert!(!uses_turns("{% for m in messages %}<start_of_turn>{{ m.role }}"));
        assert!(!uses_turns("<|im_start|>"));
        assert_eq!(
            turn_prompt(" Fix the text. \n", "ну привет "),
            "<|turn>system\nFix the text.<turn|>\n<|turn>user\nну привет<turn|>\n<|turn>model\n"
        );
    }

    /// The smoke test: a real model, `CUE_TEST_GGUF=<file>`, answers under the grammar.
    #[test]
    #[ignore = "needs a GGUF model file"]
    fn local_llm_answers_under_the_grammar() {
        let file = std::env::var("CUE_TEST_GGUF").expect("CUE_TEST_GGUF");
        let llm = LocalLlm::load(Path::new(&file), "smoke").unwrap();
        for raw in ["ну привет как дела", "hey how are you doing"] {
            let started = Instant::now();
            let deadline = Instant::now() + std::time::Duration::from_secs(120);
            let out = llm.generate("Return the user's text as written, with punctuation.", raw, TEXT_GRAMMAR, deadline, &AtomicBool::new(false)).unwrap();
            let v: serde_json::Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{out:?}: {e}"));
            eprintln!("{raw:?} -> {:?} in {} ms", v["text"], started.elapsed().as_millis());
            assert!(v["text"].as_str().is_some_and(|t| !t.is_empty()));
        }
    }

    /// The grammar's shape, as far as it can be read without llama.cpp: a root of one `text` field holding a JSON
    /// string, and nothing before or after it.
    #[test]
    fn the_grammar_is_the_text_object() {
        assert!(TEXT_GRAMMAR.starts_with("root ::= \"{\" ws \"\\\"text\\\"\" ws \":\" ws string ws \"}\""));
        assert!(TEXT_GRAMMAR.contains("string ::="));
        assert!(TEXT_GRAMMAR.is_ascii());
    }

    #[test]
    fn the_rule_grammar_has_rule_and_short() {
        assert!(RULE_GRAMMAR.starts_with("root ::= \"{\" ws \"\\\"rule\\\"\" ws \":\" ws string ws \",\" ws \"\\\"short\\\"\""));
        assert_eq!(RULE_GRAMMAR.lines().nth(2), TEXT_GRAMMAR.lines().nth(2), "the same string rule");
    }
}

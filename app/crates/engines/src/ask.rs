//! What an "Ask" question brings back, whatever answers it (docs/specs/2026-11-08-no-keys-design.md, "Ask and
//! translation"): the steps for the answer on screen, how the question ended, and its deadlines. The question goes
//! through the cleanup's route, the user's agent or the model on this computer (`agent::ask::run`). No text of the
//! question, the answer, the selection or the sources reaches the log here.
use std::time::Duration;

use cue_core::ask as core_ask;
use cue_core::flow::AgentProblem;

/// What the answer brought, for the answer on screen (spec 2 §7: the thinking line names only real steps).
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// The question is answered without web search: it is off, or the AI cannot search. Comes first.
    NoSearch,
    /// The AI started a web search.
    Searching,
    /// Search results came in.
    Reading,
    /// A text block started.
    TextStart,
    /// More text of the current text block.
    Text(String),
    /// A source of the current text block: its URL and title.
    Cite { url: String, title: String },
    /// The AI wrote text for the field: a write, not an answer. Counts as its first words.
    WriteStart,
    /// What the question has cost so far, for a route that reports it as it goes.
    Spent(AskUsage),
}

impl Step {
    /// The first words of a question: text that is not blank, or the start of a write (decision 6).
    pub fn is_first_words(&self) -> bool {
        matches!(self, Step::Text(t) if !t.trim().is_empty()) || *self == Step::WriteStart
    }
}

/// Tokens and searches a question used (usage.csv). An agent's tokens are its own business: only its searches count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AskUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub web_searches: u32,
}

impl AskUsage {
    pub fn add(&mut self, o: AskUsage) {
        self.input_tokens += o.input_tokens;
        self.output_tokens += o.output_tokens;
        self.cache_read_tokens += o.cache_read_tokens;
        self.cache_write_tokens += o.cache_write_tokens;
        self.web_searches += o.web_searches;
    }
}

/// Why a question got no answer. Never any text of the question or the answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AskFailure {
    /// No AI chosen (no AI, or nothing chosen): "Ask" is unavailable.
    NoAi,
    /// The user's agent needs them: not installed, or signed out.
    Agent(AgentProblem),
    /// No answer in time.
    Timeout,
    /// Anything else, in a word for the log.
    Failed(&'static str),
}

/// How a question ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskEnd {
    Answer,
    Write { text: String, replace: bool },
    /// The AI declined to answer: nothing to paste.
    Refused,
    Failed(AskFailure),
}

/// The end of a question, the answer's text for the conversation (the text it wrote, for a write; empty after a
/// failure), and what it used.
#[derive(Clone, Debug, PartialEq)]
pub struct AskResult {
    pub end: AskEnd,
    pub text: String,
    pub usage: AskUsage,
}

impl AskResult {
    /// A question that failed before or instead of an answer; one log line with the reason, never a text.
    pub fn failed(why: AskFailure) -> AskResult {
        tracing::warn!(reason = ?why, "ask: no answer");
        AskResult { end: AskEnd::Failed(why), text: String::new(), usage: AskUsage::default() }
    }
}

/// The deadlines of one question: its first words without and with web search, and the whole answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub first_words: Duration,
    pub first_words_search: Duration,
    pub total: Duration,
}

impl Limits {
    pub const SPEC: Limits = Limits {
        first_words: Duration::from_millis(core_ask::FIRST_WORDS_MS),
        first_words_search: Duration::from_millis(core_ask::FIRST_WORDS_SEARCH_MS),
        total: Duration::from_millis(core_ask::ANSWER_MS),
    };

    /// These deadlines with `extra` more for the first words: an agent's start allowance.
    pub fn later_by(self, extra: Duration) -> Limits {
        Limits { first_words: self.first_words + extra, first_words_search: self.first_words_search + extra, ..self }
    }
}

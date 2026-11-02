//! The connections the cleanup can go through and their models (docs/specs/2026-11-02-refine-connections.md): the
//! Anthropic Messages API, OpenRouter, and eight services that speak OpenAI's Chat Completions, two of them local
//! servers. The first model of each is the recommended one. What a request must send to each service (the answer
//! format, how reasoning is turned down, the token field, cache hints) is kept here too, beside the service.

/// How CUE talks to a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The Messages API (`cue_engines::claude`).
    Anthropic,
    /// OpenRouter's Chat Completions with its routing (`cue_engines::openrouter`).
    OpenRouter,
    /// Any other OpenAI-compatible Chat Completions (`cue_engines::chat`).
    Chat,
}

/// How the answer is held to `{"text": …}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// `response_format: json_schema` with the schema; `strict` where the service honours it.
    Schema { strict: bool },
    /// `response_format: json_object`: the service has no schema (DeepSeek) or cannot stream one (Groq), so the system
    /// message also says how the object looks.
    Object,
}

/// A hint that keeps the prompt in the service's cache.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheHint {
    /// The body's `prompt_cache_key` (Mistral).
    BodyKey,
    /// The `x-grok-conv-id` header (xAI).
    GrokHeader,
}

/// What a model is shown with under its name: its price per million tokens (input, output, a cache read), or for a
/// local model its size on disk.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cost {
    Usd { input: f64, output: f64, cache_read: f64 },
    /// Gigabytes on disk; running it costs nothing.
    Disk(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Model {
    /// The id the service takes.
    pub id: &'static str,
    /// The name the tray shows.
    pub name: &'static str,
    pub cost: Cost,
}

#[derive(Debug, PartialEq)]
pub struct Connection {
    /// The id in settings.json and Credential Manager.
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// A server on this computer: an address instead of a key.
    pub local: bool,
    /// The base address of the API (for a local server, its default).
    pub base_url: &'static str,
    /// The first is the recommended one.
    pub models: &'static [Model],
    pub answer: Answer,
    /// `reasoning_effort` (Chat connections); `None` — not sent.
    pub reasoning: Option<&'static str>,
    /// Reasoning cannot be turned off: the request gets more time and token headroom.
    pub reasons: bool,
    /// The token budget goes in `max_completion_tokens` (OpenAI) rather than `max_tokens`.
    pub completion_tokens: bool,
    /// `stream_options: {include_usage: true}`: without it the stream reports no tokens.
    pub include_usage: bool,
    pub cache_hint: Option<CacheHint>,
}

const fn usd(input: f64, output: f64) -> Cost {
    Cost::Usd { input, output, cache_read: input / 10.0 }
}

const CHAT: Connection = Connection {
    id: "",
    name: "",
    kind: Kind::Chat,
    local: false,
    base_url: "",
    models: &[],
    answer: Answer::Schema { strict: false },
    reasoning: Some("none"),
    reasons: false,
    completion_tokens: false,
    include_usage: true,
    cache_hint: None,
};

pub const CONNECTIONS: [Connection; 10] = [
    Connection {
        id: "anthropic",
        name: "Anthropic",
        kind: Kind::Anthropic,
        base_url: "https://api.anthropic.com/v1",
        models: &[
            Model { id: "claude-haiku-5-5", name: "Claude Haiku 5.5", cost: usd(0.10, 0.50) },
            Model { id: "claude-sonnet-5-5", name: "Claude Sonnet 5.5", cost: usd(2.0, 10.0) },
        ],
        answer: Answer::Schema { strict: true },
        reasoning: None,
        include_usage: false,
        ..CHAT
    },
    Connection {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        models: &[
            Model { id: "gpt-6-luna", name: "GPT-6 Luna", cost: usd(0.10, 0.50) },
            Model { id: "gpt-6.1-terra", name: "GPT-6.1 Terra", cost: usd(1.0, 5.0) },
        ],
        answer: Answer::Schema { strict: true },
        completion_tokens: true,
        ..CHAT
    },
    Connection {
        id: "gemini",
        name: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        models: &[
            Model { id: "gemini-3.1-flash-lite", name: "Gemini 3.1 Flash-Lite", cost: usd(0.25, 1.50) },
            Model { id: "gemini-3.5-flash-lite", name: "Gemini 3.5 Flash-Lite", cost: usd(0.30, 2.50) },
        ],
        reasoning: Some("minimal"),
        reasons: true,
        ..CHAT
    },
    Connection {
        id: "openrouter",
        name: "OpenRouter",
        kind: Kind::OpenRouter,
        base_url: "https://openrouter.ai/api/v1",
        models: &[
            Model { id: "anthropic/claude-haiku-5.5", name: "Claude Haiku 5.5", cost: usd(0.10, 0.50) },
            Model { id: "anthropic/claude-sonnet-5.5", name: "Claude Sonnet 5.5", cost: usd(2.0, 10.0) },
            Model { id: "openai/gpt-6-luna", name: "GPT-6 Luna", cost: usd(0.10, 0.50) },
        ],
        answer: Answer::Schema { strict: true },
        reasoning: None,
        include_usage: false,
        ..CHAT
    },
    Connection {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com",
        models: &[Model { id: "deepseek-flash", name: "DeepSeek Flash", cost: usd(0.30, 1.20) }],
        answer: Answer::Object,
        ..CHAT
    },
    Connection {
        id: "mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        models: &[
            Model { id: "mistral-small-2603", name: "Mistral Small 4", cost: usd(0.15, 0.60) },
            Model { id: "ministral-8b-2512", name: "Ministral 8B", cost: usd(0.15, 0.15) },
        ],
        include_usage: false,
        cache_hint: Some(CacheHint::BodyKey),
        ..CHAT
    },
    Connection {
        id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        models: &[
            Model { id: "openai/gpt-oss-120b", name: "gpt-oss 120B", cost: usd(0.15, 0.60) },
            Model { id: "openai/gpt-oss-20b", name: "gpt-oss 20B", cost: usd(0.075, 0.30) },
        ],
        answer: Answer::Object,
        reasoning: Some("low"),
        reasons: true,
        ..CHAT
    },
    Connection {
        id: "xai",
        name: "xAI",
        base_url: "https://api.x.ai/v1",
        models: &[Model { id: "grok-4.3", name: "Grok 4.3", cost: usd(1.25, 2.50) }],
        answer: Answer::Schema { strict: true },
        include_usage: false,
        cache_hint: Some(CacheHint::GrokHeader),
        ..CHAT
    },
    Connection {
        id: "ollama",
        name: "Ollama",
        local: true,
        base_url: "http://localhost:11434/v1",
        models: &[
            Model { id: "gemma4:12b", name: "Gemma 4 12B", cost: Cost::Disk(8.0) },
            Model { id: "gemma4:e4b", name: "Gemma 4 E4B", cost: Cost::Disk(7.0) },
        ],
        ..CHAT
    },
    Connection {
        id: "lmstudio",
        name: "LM Studio",
        local: true,
        base_url: "http://localhost:1234/v1",
        models: &[
            Model { id: "google/gemma-4-12b", name: "Gemma 4 12B", cost: Cost::Disk(7.4) },
            Model { id: "google/gemma-4-e4b", name: "Gemma 4 E4B", cost: Cost::Disk(5.9) },
        ],
        reasoning: None,
        ..CHAT
    },
];

/// The connection a settings file names by default, and in place of one CUE does not know.
pub const DEFAULT_ID: &str = "anthropic";

pub fn find(id: &str) -> Option<&'static Connection> {
    CONNECTIONS.iter().find(|c| c.id == id)
}

/// The default connection.
pub fn default() -> &'static Connection {
    find(DEFAULT_ID).expect("the default connection is listed")
}

impl Connection {
    pub fn recommended(&self) -> &'static Model {
        &self.models[0]
    }

    pub fn model(&self, id: &str) -> Option<&'static Model> {
        self.models.iter().find(|m| m.id == id)
    }

    /// The model's id in usage.csv and the price table: the Messages API's and OpenRouter's ids as they are (they
    /// already name the maker), any other one behind the connection (`openai:gpt-6-luna`).
    pub fn usage_id(&self, model: &Model) -> String {
        match self.kind {
            Kind::Anthropic | Kind::OpenRouter => model.id.to_owned(),
            Kind::Chat => format!("{}:{}", self.id, model.id),
        }
    }
}

/// Every listed model as the price table counts it: usage id, input, output, cache read (dollars per million
/// tokens); a local model costs nothing.
pub fn prices() -> impl Iterator<Item = (String, f64, f64, f64)> {
    CONNECTIONS.iter().flat_map(|c| {
        c.models.iter().map(move |m| {
            let (input, output, cache_read) = match m.cost {
                Cost::Usd { input, output, cache_read } => (input, output, cache_read),
                Cost::Disk(_) => (0.0, 0.0, 0.0),
            };
            (c.usage_id(m), input, output, cache_read)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_connections_two_of_them_local() {
        let ids: Vec<_> = CONNECTIONS.iter().map(|c| c.id).collect();
        assert_eq!(ids, ["anthropic", "openai", "gemini", "openrouter", "deepseek", "mistral", "groq", "xai", "ollama", "lmstudio"]);
        let local: Vec<_> = CONNECTIONS.iter().filter(|c| c.local).map(|c| c.id).collect();
        assert_eq!(local, ["ollama", "lmstudio"]);
        assert_eq!(default().id, "anthropic");
        assert_eq!(find("openai").unwrap().name, "OpenAI");
        assert!(find("bard").is_none());
    }

    #[test]
    fn every_connection_has_a_recommended_model_and_unique_ids() {
        for c in &CONNECTIONS {
            assert!(!c.models.is_empty(), "{}", c.id);
            assert_eq!(c.model(c.recommended().id), Some(c.recommended()));
            let mut ids: Vec<_> = c.models.iter().map(|m| m.id).collect();
            ids.dedup();
            assert_eq!(ids.len(), c.models.len(), "{}", c.id);
            assert!(c.base_url.starts_with(if c.local { "http://localhost:" } else { "https://" }), "{}", c.id);
            assert!(!c.base_url.ends_with('/'), "{}", c.id);
            // A cloud model has a price, a local one a size.
            for m in c.models {
                assert_eq!(matches!(m.cost, Cost::Disk(_)), c.local, "{} {}", c.id, m.id);
            }
        }
        assert_eq!(default().recommended().id, "claude-haiku-5-5");
        assert_eq!(find("openrouter").unwrap().recommended().id, "anthropic/claude-haiku-5.5");
    }

    #[test]
    fn what_each_service_needs_in_a_request() {
        let c = |id| find(id).unwrap();
        assert_eq!(c("deepseek").answer, Answer::Object);
        assert_eq!(c("groq").answer, Answer::Object);
        assert_eq!(c("openai").answer, Answer::Schema { strict: true });
        assert!(c("openai").completion_tokens && !c("gemini").completion_tokens);
        assert_eq!((c("gemini").reasoning, c("gemini").reasons), (Some("minimal"), true));
        assert_eq!((c("groq").reasoning, c("groq").reasons), (Some("low"), true));
        assert_eq!((c("deepseek").reasoning, c("deepseek").reasons), (Some("none"), false));
        assert_eq!(c("lmstudio").reasoning, None);
        assert_eq!(c("mistral").cache_hint, Some(CacheHint::BodyKey));
        assert_eq!(c("xai").cache_hint, Some(CacheHint::GrokHeader));
        assert_eq!(c("anthropic").kind, Kind::Anthropic);
        assert_eq!(c("openrouter").kind, Kind::OpenRouter);
    }

    #[test]
    fn usage_ids_name_the_connection_where_the_model_does_not() {
        let c = |id| find(id).unwrap();
        assert_eq!(c("anthropic").usage_id(c("anthropic").recommended()), "claude-haiku-5-5");
        assert_eq!(c("openrouter").usage_id(c("openrouter").recommended()), "anthropic/claude-haiku-5.5");
        assert_eq!(c("openai").usage_id(c("openai").recommended()), "openai:gpt-6-luna");
        assert_eq!(c("groq").usage_id(c("groq").recommended()), "groq:openai/gpt-oss-120b");
        let prices: Vec<_> = prices().collect();
        assert!(prices.contains(&("openai:gpt-6-luna".to_owned(), 0.10, 0.50, 0.01)));
        assert!(prices.contains(&("ollama:gemma4:12b".to_owned(), 0.0, 0.0, 0.0)));
        let mut ids: Vec<_> = prices.iter().map(|p| p.0.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), prices.len(), "every usage id is one model");
    }
}

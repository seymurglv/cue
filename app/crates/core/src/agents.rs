//! The AI agents CUE can clean text through (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): the user's own
//! OpenCode, Codex or Grok, signed in by the user. CUE holds no key for any of them. Claude Code and Gemini /
//! Antigravity are never offered: their makers forbid a third-party app driving a user's subscription.
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An agent CUE knows. Every id stays here so settings that name it still parse, even while [`ORDER`] leaves it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AgentId {
    OpenCode,
    Codex,
    Grok,
}

/// The agents CUE offers, in the order every list shows them.
///
/// Grok is left out on grok 1.0.34: its headless mode (`grok -p`) takes the prompt only as a command-line argument or
/// a file and does not read stdin, so the dictation would be visible in the process list or written to disk (the
/// spec's anti-goal). The evidence is in `app/crates/engines/tests/fixtures/agents/README.md`.
pub const ORDER: &[AgentId] = &[AgentId::OpenCode, AgentId::Codex];

/// The words in a model's name that mark a small fast one, in the order OpenCode's "Быстрая · авто" prefers them.
pub const FAST_NAMES: [&str; 7] = ["haiku", "flash", "mini", "lite", "luna", "fast", "small"];

/// Codex's models (`codex debug models`, codex-cli 0.166.0, the ones it lists), the default first.
const CODEX_MODELS: &[(&str, &str)] = &[
    ("gpt-6-luna", "GPT-6 Luna"),
    ("gpt-6-sol", "GPT-6 Sol"),
    ("gpt-6-astra", "GPT-6 Astra"),
    ("gpt-6.1-sol", "GPT-6.1 Sol"),
    ("gpt-6.1-terra", "GPT-6.1 Terra"),
    ("gpt-5.6-luna", "GPT-5.6 Luna"),
];

/// Grok's models (`grok models`, grok 1.0.34), its default first.
const GROK_MODELS: &[(&str, &str)] = &[("grok-4.6", "Grok 4.6"), ("grok-4.5", "Grok 4.5")];

impl AgentId {
    /// The id settings keep.
    pub fn id(&self) -> &'static str {
        match self {
            AgentId::OpenCode => "opencode",
            AgentId::Codex => "codex",
            AgentId::Grok => "grok",
        }
    }

    /// The name the window shows.
    pub fn name(&self) -> &'static str {
        match self {
            AgentId::OpenCode => "OpenCode",
            AgentId::Codex => "Codex",
            AgentId::Grok => "Grok",
        }
    }

    pub fn find(id: &str) -> Option<AgentId> {
        [AgentId::OpenCode, AgentId::Codex, AgentId::Grok].into_iter().find(|a| a.id() == id)
    }

    /// The models CUE offers for the agent as (id, name), the default first. Empty for OpenCode, which lists its own.
    pub fn models(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            AgentId::OpenCode => &[],
            AgentId::Codex => CODEX_MODELS,
            AgentId::Grok => GROK_MODELS,
        }
    }
}

impl Serialize for AgentId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.id())
    }
}

impl<'de> Deserialize<'de> for AgentId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let id = String::deserialize(d)?;
        AgentId::find(&id).ok_or_else(|| serde::de::Error::custom(format!("unknown agent {id:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_claude_is_unknown() {
        for a in [AgentId::OpenCode, AgentId::Codex, AgentId::Grok] {
            assert_eq!(AgentId::find(a.id()), Some(a));
            assert_eq!(serde_json::to_string(&a).unwrap(), format!("\"{}\"", a.id()));
            assert_eq!(serde_json::from_str::<AgentId>(&format!("\"{}\"", a.id())).unwrap(), a);
        }
        assert_eq!(AgentId::find("claude"), None);
        assert_eq!(AgentId::find("gemini"), None);
        assert!(serde_json::from_str::<AgentId>("\"claude\"").is_err());
    }

    #[test]
    fn order_starts_with_opencode_then_codex() {
        assert_eq!(&ORDER[..2], &[AgentId::OpenCode, AgentId::Codex]);
        // Grok keeps its place after Codex whenever it is offered again.
        assert!(ORDER.iter().all(|a| *a != AgentId::Grok) || ORDER.last() == Some(&AgentId::Grok));
    }

    #[test]
    fn default_models() {
        assert_eq!(AgentId::Codex.models()[0], ("gpt-6-luna", "GPT-6 Luna"));
        assert_eq!(AgentId::Grok.models()[0].0, "grok-4.6");
        assert!(AgentId::OpenCode.models().is_empty());
        assert_eq!(FAST_NAMES, ["haiku", "flash", "mini", "lite", "luna", "fast", "small"]);
    }
}

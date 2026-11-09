//! The AI agents CUE can clean text through (docs/specs/2026-11-08-no-keys-design.md, "Your AI"): the user's own
//! OpenCode or Codex, signed in by the user. CUE holds no key for any of them. Claude Code and Gemini /
//! Antigravity are never offered: their makers forbid a third-party app driving a user's subscription.
use std::path::{Path, PathBuf};

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
/// Grok is left out (the owner's decision of 09.11.2026): the only Grok model that cleans dictations without acting on
/// or commenting on them, grok-4.7, takes 6–9 s for a short cleanup. A named-pipe route that kept the text off the
/// command line and the disk was built and reverted; the evidence is in
/// `app/crates/engines/tests/fixtures/agents/README.md`.
pub const ORDER: &[AgentId] = &[AgentId::OpenCode, AgentId::Codex];

/// The words in a model's name that mark a small fast one, in the order OpenCode's "Быстрая · авто" prefers them.
pub const FAST_NAMES: [&str; 7] = ["haiku", "flash", "mini", "lite", "luna", "fast", "small"];

/// Codex's models as codex-cli 0.166.0 listed them on 09.11.2026 (`codex debug models`), the default first: what CUE
/// offers only when Codex's own list of the user's models ([`codex_models`]) cannot be read.
const CODEX_MODELS: &[(&str, &str)] = &[
    ("gpt-6-luna", "GPT-6 Luna"),
    ("gpt-6-sol", "GPT-6 Sol"),
    ("gpt-6-astra", "GPT-6 Astra"),
    ("gpt-6.1-sol", "GPT-6.1 Sol"),
    ("gpt-6.1-terra", "GPT-6.1 Terra"),
    ("gpt-5.6-luna", "GPT-5.6 Luna"),
];

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

    /// The models CUE knows for the agent without asking it, as (id, name), the default first: Codex's fixed list, the
    /// fallback of [`codex_models`]. Empty for OpenCode, which lists its own, and for Grok, which is not offered.
    pub fn models(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            AgentId::Codex => CODEX_MODELS,
            AgentId::OpenCode | AgentId::Grok => &[],
        }
    }

    /// Whether CUE offers this agent ([`ORDER`]): a settings file that names another one reads as naming none.
    pub fn offered(&self) -> bool {
        ORDER.contains(self)
    }
}

/// A model id that may go on an agent's command line: letters, digits and `.-_:/`, at most 64 (`cmd.exe` may parse it).
pub fn plain_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 64
        && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/'))
}

/// Codex's list of the user's models: `models_cache.json` in Codex's folder (`CODEX_HOME`, else `%USERPROFILE%\.codex`),
/// which Codex itself keeps from the user's account. None when neither folder is known.
pub fn codex_models_file() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".codex")))?;
    Some(home.join("models_cache.json"))
}

/// The models Codex lists for the user's account, as (id, name), its default first ([`codex_models_in`] of
/// [`codex_models_file`]); CUE's fixed list when the file is missing, unreadable or lists none. Read when asked: no
/// process, no network.
pub fn codex_models() -> Vec<(String, String)> {
    codex_models_at(codex_models_file().as_deref())
}

/// [`codex_models`] from the file at `path`.
pub fn codex_models_at(path: Option<&Path>) -> Vec<(String, String)> {
    path.and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| codex_models_in(&bytes))
        .unwrap_or_else(|| CODEX_MODELS.iter().map(|(id, name)| ((*id).to_owned(), (*name).to_owned())).collect())
}

/// The models of Codex's `models_cache.json` (codex-cli 0.166.0: `{"models": [{"slug", "display_name", "visibility",
/// "priority", …}]}`) that Codex shows in its own list (`visibility` `list`; hidden ones, such as its reviewer, are left
/// out), by `priority` (Codex's default first), as (slug, display name). Only a slug that may go on a command line
/// ([`plain_model`]) is taken. None when the text is not of that shape or lists none.
pub fn codex_models_in(bytes: &[u8]) -> Option<Vec<(String, String)>> {
    #[derive(Deserialize)]
    struct Cache {
        models: Vec<serde_json::Value>,
    }
    let cache: Cache = serde_json::from_slice(bytes).ok()?;
    let mut listed: Vec<(i64, String, String)> = cache
        .models
        .iter()
        .filter(|m| m["visibility"].as_str() == Some("list"))
        .filter_map(|m| {
            let slug = m["slug"].as_str().filter(|s| plain_model(s))?;
            let name = m["display_name"].as_str().map(str::trim).filter(|n| !n.is_empty()).unwrap_or(slug);
            Some((m["priority"].as_i64().unwrap_or(i64::MAX), slug.to_owned(), name.to_owned()))
        })
        .collect();
    listed.sort_by_key(|(priority, _, _)| *priority);
    let mut models: Vec<(String, String)> = Vec::with_capacity(listed.len());
    for (_, id, name) in listed {
        if !models.iter().any(|(m, _)| *m == id) {
            models.push((id, name));
        }
    }
    (!models.is_empty()).then_some(models)
}

/// The model to ask of a `listed` set (the default first): `saved` while it is listed, else the default. A saved model
/// the list no longer holds falls back with no error. Empty when nothing is listed.
pub fn listed_or_default<'a>(listed: &'a [(String, String)], saved: Option<&str>) -> &'a str {
    saved
        .and_then(|m| listed.iter().find(|(id, _)| id == m))
        .or_else(|| listed.first())
        .map_or("", |(id, _)| id.as_str())
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
    fn order_is_opencode_then_codex() {
        assert_eq!(ORDER, &[AgentId::OpenCode, AgentId::Codex]);
        assert!(AgentId::OpenCode.offered() && AgentId::Codex.offered());
        assert!(!AgentId::Grok.offered());
    }

    #[test]
    fn default_models() {
        assert_eq!(AgentId::Codex.models()[0], ("gpt-6-luna", "GPT-6 Luna"));
        assert!(AgentId::Grok.models().is_empty(), "Grok is not offered");
        assert!(AgentId::OpenCode.models().is_empty());
        assert_eq!(FAST_NAMES, ["haiku", "flash", "mini", "lite", "luna", "fast", "small"]);
    }

    fn temp_file(name: &str, text: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cue-codex-models-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("models_cache.json");
        std::fs::write(&file, text).unwrap();
        file
    }

    /// The shape of codex-cli 0.166.0's `models_cache.json` (the fields CUE reads, most others cut), for an account with
    /// GPT-6 Luna and the 5.6 models: hidden models left out, the rest by priority.
    const CACHE: &str = r#"{"fetched_at":"2026-11-09T08:10:35Z","client_version":"0.166.0","models":[
        {"slug":"gpt-reserve","display_name":"GPT-Reserve","visibility":"hide","priority":1,"context_window":1},
        {"slug":"gpt-5.6-luna","display_name":"GPT-5.6-Luna","visibility":"list","priority":4},
        {"slug":"gpt-6-luna","display_name":"GPT-6-Luna","visibility":"list","priority":2,"upgrade":null},
        {"slug":"gpt-5.6-terra","display_name":"GPT-5.6-Terra","visibility":"list","priority":3},
        {"slug":"codex-auto-review","display_name":"Codex Auto Review","visibility":"hide","priority":5},
        {"slug":"bad model & calc","display_name":"Bad","visibility":"list","priority":6},
        {"slug":"gpt-unnamed","visibility":"list","priority":7},
        {"slug":"gpt-6-luna","display_name":"Again","visibility":"list","priority":8}
    ]}"#;

    #[test]
    fn codex_lists_the_accounts_models_from_its_cache() {
        let file = temp_file("cache", CACHE);
        let models = codex_models_at(Some(&file));
        let ids: Vec<&str> = models.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["gpt-6-luna", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-unnamed"]);
        assert_eq!(models[0], ("gpt-6-luna".to_owned(), "GPT-6-Luna".to_owned()));
        assert_eq!(models[3].1, "gpt-unnamed", "a model with no name shows its id");
        let _ = std::fs::remove_dir_all(file.parent().unwrap());
    }

    /// No file, a broken one, another shape, or one that lists nothing: CUE's fixed list.
    #[test]
    fn codex_falls_back_to_the_fixed_list() {
        let fixed: Vec<(String, String)> = CODEX_MODELS.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect();
        assert_eq!(codex_models_at(None), fixed);
        let missing = std::env::temp_dir().join(format!("cue-codex-models-{}-none", std::process::id())).join("models_cache.json");
        assert_eq!(codex_models_at(Some(&missing)), fixed);
        for text in ["{not json", "[]", r#"{"models":{}}"#, r#"{"models":[]}"#, r#"{"models":[{"slug":"x","visibility":"hide"}]}"#] {
            let file = temp_file("bad", text);
            assert_eq!(codex_models_at(Some(&file)), fixed, "{text}");
            let _ = std::fs::remove_dir_all(file.parent().unwrap());
        }
    }

    #[test]
    fn a_saved_model_is_asked_while_it_is_listed() {
        let listed = codex_models_in(CACHE.as_bytes()).unwrap();
        assert_eq!(listed_or_default(&listed, Some("gpt-5.6-terra")), "gpt-5.6-terra");
        assert_eq!(listed_or_default(&listed, Some("gpt-6-sol")), "gpt-6-luna", "no longer listed: the default");
        assert_eq!(listed_or_default(&listed, None), "gpt-6-luna");
        assert_eq!(listed_or_default(&[], Some("gpt-6-luna")), "");
    }

    #[test]
    fn plain_model_ids_only() {
        for ok in ["gpt-6-luna", "gpt-5.6-terra", "anthropic/claude-haiku-5-5", "a:b_c"] {
            assert!(plain_model(ok), "{ok}");
        }
        let long = "m".repeat(65);
        for bad in ["", "gpt\" & calc", "a b", "x%PATH%", "m\n-c", long.as_str()] {
            assert!(!plain_model(bad), "{bad:?}");
        }
    }
}

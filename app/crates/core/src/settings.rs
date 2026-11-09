//! settings.json: engine and models, refine thresholds, the hint counter, bench, interface language, where the Drop
//! sits, the user's dictionary, the habits, the translation pair, "Ask", the pinned microphone, how long the history keeps its days
//! and their recordings. The main window changes a setting one at a time
//! (`SettingChange`).
use crate::agents::AgentId;
use crate::habits::Habits;
use crate::refine;
use crate::translate;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Transcription engine: Parakeet on this computer, the only one. The type stays for the `stt.engine` field in
/// settings.json and the `stt_engine` column in usage.csv. Any other name in the file (Soniox and ElevenLabs of earlier
/// versions) reads as Parakeet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SttEngineKind {
    #[default]
    #[serde(other)]
    Parakeet,
}

/// The fields of earlier engines (`soniox_model`, `language_hints` and the rest) are read past and never written again.
/// `model` — the speech model's id (`crate::model`), written once a newly chosen one has loaded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SttSettings {
    pub engine: SttEngineKind,
    pub model: String,
}

impl Default for SttSettings {
    fn default() -> Self {
        Self { engine: SttEngineKind::default(), model: crate::model::DEFAULT_ID.to_owned() }
    }
}

/// The longest custom instructions the cleanup takes, in characters.
pub const MAX_CUSTOM_CHARS: usize = 2_000;

/// The cleanup: where it runs, which agent and model, and what happens when the AI does not answer
/// (docs/specs/2026-11-08-no-keys-design.md, "Settings and migration"). The `runs`, `style`, `custom` and `connection`
/// of earlier versions, the old `model`, `level` and `addresses` are read once for the move and never written again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredRefine")]
pub struct RefineSettings {
    /// No AI, a model on this computer, or the user's own agent.
    #[serde(rename = "where")]
    pub where_: Where,
    /// The local model's id; empty: nothing chosen.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub local: String,
    /// The agent chosen; `None`: the first one found, resolved when CUE starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentId>,
    /// The model the user picked per agent id, only where it is not the agent's default.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub models: BTreeMap<String, String>,
    /// What to paste when the AI does not answer.
    pub fallback: Fallback,
    /// How many characters before the caret to give the model.
    pub context_chars: usize,
    /// Sanity check thresholds.
    pub min_ratio: f32,
    pub max_ratio: f32,
    pub min_overlap: f32,
    /// The text of the old custom style, read for the move into the instructions file. It is written back as the old
    /// `style: "custom"` and `custom` until the runtime has seen `instructions.md` exist and clears it, so a failed
    /// first write of that file never loses the user's text.
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    pub legacy_custom: Option<LegacyCustom>,
}

/// The old custom style as settings.json held it: `{"style": "custom", "custom": text}`.
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyCustom(pub String);

impl Serialize for LegacyCustom {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut o = s.serialize_struct("LegacyCustom", 2)?;
        o.serialize_field("style", "custom")?;
        o.serialize_field("custom", &self.0)?;
        o.end()
    }
}

/// Where the cleanup runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Where {
    /// No AI: CUE's own tidy-up only.
    Off,
    /// A model on this computer.
    Local,
    /// The user's own agent.
    #[default]
    Agent,
}

/// `"off"`, `"local"` and `"agent"` (case and surrounding spaces do not matter); anything else reads as `Agent`.
impl<'de> Deserialize<'de> for Where {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let name = value.as_str().map(|s| s.trim().to_ascii_lowercase());
        Ok(match name.as_deref() {
            Some("off") => Self::Off,
            Some("local") => Self::Local,
            _ => Self::Agent,
        })
    }
}

/// What is pasted when the AI does not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fallback {
    /// The no-AI layer's text as it is.
    #[default]
    Raw,
    /// The local model's cleanup, when it is loaded at that moment.
    Local,
}

/// Only `"local"` is local (case and surrounding spaces do not matter); anything else is `Raw`.
impl<'de> Deserialize<'de> for Fallback {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let local = value.as_str().is_some_and(|s| s.trim().eq_ignore_ascii_case("local"));
        Ok(if local { Self::Local } else { Self::Raw })
    }
}

fn present_where<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Where>, D::Error> {
    Where::deserialize(deserializer).map(Some)
}

/// The model earlier versions recommended on this computer: a `runs: local` file with no model chosen kept it.
const OLD_RECOMMENDED_LOCAL: (&str, &str) = ("unsloth/Qwen3-4B-Instruct-2507-GGUF", "Q4_K_M");

/// The local servers of earlier versions; choosing one meant the cleanup ran on this computer.
const OLD_LOCAL_SERVERS: [&str; 2] = ["ollama", "lmstudio"];

/// `refine` as settings.json holds it, of this version or an earlier one.
#[derive(Deserialize)]
#[serde(default)]
struct StoredRefine {
    /// Absent: an earlier file, moved by `runs`; present, even `null` or nonsense: [`Where`] reads it.
    #[serde(rename = "where", deserialize_with = "present_where")]
    where_: Option<Where>,
    /// Earlier versions: `"local"` or anything else (the cloud).
    runs: serde_json::Value,
    local: String,
    agent: serde_json::Value,
    connection: serde_json::Value,
    models: BTreeMap<String, String>,
    fallback: Fallback,
    /// Earlier versions: `"custom"` or anything else.
    style: serde_json::Value,
    custom: String,
    context_chars: usize,
    min_ratio: f32,
    max_ratio: f32,
    min_overlap: f32,
}

impl Default for StoredRefine {
    fn default() -> Self {
        let d = RefineSettings::default();
        Self {
            where_: None,
            runs: serde_json::Value::Null,
            local: d.local,
            agent: serde_json::Value::Null,
            connection: serde_json::Value::Null,
            models: d.models,
            fallback: d.fallback,
            style: serde_json::Value::Null,
            custom: String::new(),
            context_chars: d.context_chars,
            min_ratio: d.min_ratio,
            max_ratio: d.max_ratio,
            min_overlap: d.min_overlap,
        }
    }
}

impl From<StoredRefine> for RefineSettings {
    fn from(s: StoredRefine) -> Self {
        let named = |v: &serde_json::Value| v.as_str().map(|id| id.trim().to_ascii_lowercase());
        let mut local = s.local.trim().to_owned();
        let where_ = s.where_.unwrap_or_else(|| {
            // An earlier file: `runs: local`, or a local server as the connection, ran on this computer; anything else
            // was the cloud, now the user's agent (which one is resolved when CUE starts).
            let old_server = named(&s.connection).is_some_and(|id| OLD_LOCAL_SERVERS.contains(&id.as_str()));
            let runs_local = named(&s.runs).map_or(old_server, |r| r == "local");
            if !runs_local {
                return Where::Agent;
            }
            if local.is_empty() {
                local = crate::llm::id_for(OLD_RECOMMENDED_LOCAL.0, OLD_RECOMMENDED_LOCAL.1);
            }
            Where::Local
        });
        let agent = named(&s.agent).and_then(|id| AgentId::find(&id));
        // The models of earlier versions were per cloud connection; only an agent's entry stays.
        // Validated as `apply` does: Codex and Grok keep only listed models other than their default, OpenCode any name
        // of at most `MAX_MODEL_CHARS` characters.
        let models = s
            .models
            .into_iter()
            .filter_map(|(id, m)| {
                let agent = AgentId::find(&id)?;
                let m = m.trim();
                let known = agent.models();
                let ok = if known.is_empty() { m.chars().count() <= MAX_MODEL_CHARS } else { known.iter().skip(1).any(|(k, _)| *k == m) };
                (!m.is_empty() && ok).then(|| (id, m.to_owned()))
            })
            .collect();
        let custom = s.custom.trim();
        let legacy_custom = (named(&s.style).as_deref() == Some("custom") && !custom.is_empty()).then(|| LegacyCustom(custom.to_owned()));
        Self {
            where_,
            local,
            agent,
            models,
            fallback: s.fallback,
            context_chars: s.context_chars,
            min_ratio: s.min_ratio,
            max_ratio: s.max_ratio,
            min_overlap: s.min_overlap,
            legacy_custom,
        }
    }
}

impl RefineSettings {
    /// The model's id in usage.csv: a local model's is `local:<id>`, an agent's `agent:<agent>:<model>` (`auto` for
    /// OpenCode's "Быстрая · авто"). No AI, or nothing chosen, is `off`.
    pub fn usage_model(&self) -> String {
        match (self.where_, self.agent) {
            (Where::Local, _) if !self.local.is_empty() => format!("local:{}", self.local),
            (Where::Agent, Some(a)) => format!("agent:{}:{}", a.id(), self.agent_model(a).unwrap_or("auto")),
            _ => "off".to_owned(),
        }
    }

    /// The model the cleanup asks agent `a` for: the one the user picked, else the agent's default (Codex's and Grok's
    /// first). `None` for OpenCode with nothing picked: "Быстрая · авто", chosen from the models the server lists.
    pub fn agent_model(&self, a: AgentId) -> Option<&str> {
        self.models.get(a.id()).map(String::as_str).or_else(|| a.models().first().map(|m| m.0))
    }

    /// The text of the old custom style, when the file had one: read once to fill the instructions file, never written.
    pub fn legacy_custom(&self) -> Option<&str> {
        self.legacy_custom.as_ref().map(|l| l.0.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchSettings {
    pub save_recordings: bool,
}

/// Interface language for the tray menu and the Drop. `Auto` follows the Windows display language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UiLanguage {
    #[default]
    Auto,
    En,
    Ru,
}

/// Only `"en"` and `"ru"` pick a language (case and surrounding spaces do not matter); any other value (`"auto"`,
/// another code, a number, `null`) reads as `Auto`, so a typo in this field never makes the whole file unreadable.
impl<'de> Deserialize<'de> for UiLanguage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let code = value.as_str().map(|s| s.trim().to_ascii_lowercase());
        Ok(match code.as_deref() {
            Some("en") => Self::En,
            Some("ru") => Self::Ru,
            _ => Self::Auto,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub language: UiLanguage,
}

/// Where the Drop sits on the work area: centered at the bottom (the default) or the top edge, or vertically centered
/// at the left or the right edge. The user picks it by dragging the Drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DropPosition {
    #[default]
    Bottom,
    Top,
    Left,
    Right,
}

impl DropPosition {
    pub const ALL: [Self; 4] = [Self::Bottom, Self::Top, Self::Left, Self::Right];

    /// The name in settings.json and in the Drop window.
    pub fn code(self) -> &'static str {
        match self {
            Self::Bottom => "bottom",
            Self::Top => "top",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// Case and surrounding spaces do not matter; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        Self::ALL.into_iter().find(|p| p.code().eq_ignore_ascii_case(text))
    }
}

/// Like `UiLanguage`: any value that is not one of the four names (another word, a number, `null`) reads as `Bottom`,
/// so a typo never makes the whole file unreadable.
impl<'de> Deserialize<'de> for DropPosition {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(value.as_str().and_then(Self::parse).unwrap_or_default())
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize)]
pub struct DropSettings {
    pub position: DropPosition,
}

/// `drop` that is not an object (a string, a number, `null`) reads as the defaults instead of failing the whole file;
/// inside it, unknown fields are ignored and a bad `position` reads as `bottom`.
impl<'de> Deserialize<'de> for DropSettings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let position = value.get("position").and_then(|v| v.as_str()).and_then(DropPosition::parse).unwrap_or_default();
        Ok(Self { position })
    }
}

/// Names and terms the user often says (product names, people, jargon), written as they should appear. Only the
/// refine request gets them, as a cached block after the system prompt (`refine::terms_prompt`, which also drops
/// unusable entries); recognition does not take them. `learned` holds the words added from a suggestion, newest
/// first; each is in `terms` too.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DictionarySettings {
    #[serde(deserialize_with = "strings_or_nothing")]
    pub terms: Vec<String>,
    #[serde(deserialize_with = "learned_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub learned: Vec<LearnedWord>,
}

/// A word added from a suggestion after a correction, with what recognition heard instead; refine is told both.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct LearnedWord {
    pub word: String,
    pub heard: String,
}

/// Either may be a name: only their lengths.
impl std::fmt::Debug for LearnedWord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LearnedWord")
            .field("word_chars", &self.word.chars().count())
            .field("heard_chars", &self.heard.chars().count())
            .finish()
    }
}

fn learned_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<LearnedWord>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else { return Ok(Vec::new()) };
    Ok(items.into_iter().filter_map(|item| serde_json::from_value(item).ok()).collect())
}

/// A list of strings, kept as written (trimming and the other rules apply on use). Anything other than a list
/// (a string, a number, `null`) reads as an empty list and items that are not strings are skipped, so a typo in this
/// field never makes the whole file unreadable.
fn strings_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else { return Ok(Vec::new()) };
    Ok(items
        .into_iter()
        .filter_map(|item| match item {
            serde_json::Value::String(s) => Some(s),
            _ => None,
        })
        .collect())
}

/// Translation (spec 2 §9). Every field is optional: missing, unknown or misspelled codes read as the default, and the
/// defaults are never written. `home` defaults to the Windows display language at run time (`translate::resolve_home`).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TranslateSettings {
    /// The language of the pair the speech goes to; default `en`.
    #[serde(deserialize_with = "target_or_nothing", skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The other language of the pair.
    #[serde(deserialize_with = "language_or_nothing", skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// The speech language, which fixes the direction of a translation; `auto` (or nothing) — the text's alphabet
    /// tells it (`speech::speech_language`).
    #[serde(deserialize_with = "language_or_nothing", skip_serializing_if = "Option::is_none")]
    pub speech: Option<String>,
}

impl TranslateSettings {
    pub fn target(&self) -> &'static str {
        self.target.as_deref().and_then(translate::known).unwrap_or(DEFAULT_TARGET)
    }

    pub fn speech(&self) -> Option<&'static str> {
        self.speech.as_deref().and_then(translate::known)
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A language code CUE knows (`translate::LANGUAGES`), normalized ("RU" → "ru"); anything else (`"auto"`, an unknown code, a number, `null`)
/// reads as nothing, so a typo never makes the file unreadable.
fn language_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_str().and_then(translate::known).map(str::to_owned))
}

/// The translation target when the settings name none.
const DEFAULT_TARGET: &str = "en";

/// Like [`language_or_nothing`], and the default target also reads as nothing: like an explicit `ui.language` `"auto"`,
/// it is not written back.
fn target_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Ok(language_or_nothing(deserializer)?.filter(|code| code != DEFAULT_TARGET))
}

/// "Learn from me": after a paste CUE reads the field to notice a correction (docs/specs/2026-11-02-corrections.md).
/// On by default; only `false` is written.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LearnSettings {
    #[serde(deserialize_with = "off_or_nothing", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl LearnSettings {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// "Ask" (spec 2 §9). Every field is optional: a missing or invalid value reads as the default, and a default — also one
/// the file states — is never written back, like `translate`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AskSettings {
    /// "Ask" as a whole (a paid feature later); on by default.
    #[serde(deserialize_with = "off_or_nothing", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Web search in answers, when the user's AI can search; on by default.
    #[serde(deserialize_with = "off_or_nothing", skip_serializing_if = "Option::is_none")]
    pub web_search: Option<bool>,
    #[serde(deserialize_with = "searches_or_nothing", skip_serializing_if = "Option::is_none")]
    pub max_searches: Option<u32>,
}

const DEFAULT_MAX_SEARCHES: u32 = 2;
/// More searches a question than this is a typo: each one costs money.
const MAX_SEARCHES_LIMIT: u32 = 10;

impl AskSettings {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn web_search(&self) -> bool {
        self.web_search.unwrap_or(true)
    }

    pub fn max_searches(&self) -> u32 {
        self.max_searches.unwrap_or(DEFAULT_MAX_SEARCHES)
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// `false` turns a switch off; `true` (the default) and anything else (a string, a number, `null`) read as nothing.
fn off_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<bool>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok((value == serde_json::Value::Bool(false)).then_some(false))
}

/// A whole number of searches from 1 to `MAX_SEARCHES_LIMIT`; anything else, or the default, reads as nothing.
fn searches_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .filter(|n| (1..=u64::from(MAX_SEARCHES_LIMIT)).contains(n))
        .map(|n| n as u32)
        .filter(|&n| n != DEFAULT_MAX_SEARCHES))
}

/// A section that is not an object (`"translate": "en"`, a number) reads as the default section instead of making the
/// whole file unreadable.
fn section_or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// The microphone CUE records from (spec 3 §3). Not written while it follows Windows.
#[derive(Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    /// The device's name as Windows shows it; `None` — the microphone Windows uses by default.
    #[serde(deserialize_with = "device_or_nothing", skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
}

impl AudioSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// A device name is the user's (it may carry a person's name, like "Jane's AirPods"): a log shows only its length.
impl std::fmt::Debug for AudioSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioSettings").field("device_chars", &self.device.as_ref().map(|d| d.chars().count())).finish()
    }
}

/// A device name, trimmed; not a string, or blank, reads as nothing (the Windows microphone), so a typo never makes
/// the file unreadable.
fn device_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_str().map(str::trim).filter(|d| !d.is_empty()).map(str::to_owned))
}

/// A longer microphone name from the window is refused: Windows names are far shorter, so it can only be a mistake.
const MAX_DEVICE_CHARS: usize = 200;
/// The longest agent model name a change takes.
const MAX_MODEL_CHARS: usize = 200;

/// How long the history keeps its days (spec 3 §2); `history::prune` deletes the older ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Keep {
    Forever,
    Year,
    #[default]
    Month,
    Week,
    Off,
}

impl Keep {
    pub const ALL: [Self; 5] = [Self::Forever, Self::Year, Self::Month, Self::Week, Self::Off];

    /// The name in settings.json and in the main window.
    pub fn code(self) -> &'static str {
        match self {
            Self::Forever => "forever",
            Self::Year => "year",
            Self::Month => "month",
            Self::Week => "week",
            Self::Off => "off",
        }
    }

    /// Case and surrounding spaces do not matter; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        Self::ALL.into_iter().find(|k| k.code().eq_ignore_ascii_case(text))
    }

    /// What `history::prune` deletes of the days: a week keeps the 7 days before today, a month 31, a year 366; `None`
    /// — forever, nothing.
    pub fn cutoff(self) -> Option<Cutoff> {
        match self {
            Self::Forever => None,
            Self::Year => Some(Cutoff::DaysBack(366)),
            Self::Month => Some(Cutoff::DaysBack(31)),
            Self::Week => Some(Cutoff::DaysBack(7)),
            Self::Off => Some(Cutoff::All),
        }
    }
}

/// What a period deletes when the history is cleaned up (`Keep::cutoff`, `AudioKeep::cutoff`): every day or recording,
/// or those dated before the `n` days before today — 0 keeps today's alone. One dated after today (the clock was moved
/// back) is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cutoff {
    All,
    DaysBack(u32),
}

/// How long the history keeps the recordings of its entries; `history::prune_audio` deletes the older ones. A day keeps
/// today's recordings; a week and a month count back like the history's (`Keep`). Declared from the shortest, so of
/// two periods the shorter is the lesser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioKeep {
    Off,
    Day,
    #[default]
    Week,
    Month,
}

impl AudioKeep {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::Off];

    /// The name in settings.json and in the main window.
    pub fn code(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Off => "off",
        }
    }

    /// Case and surrounding spaces do not matter; anything else is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        Self::ALL.into_iter().find(|k| k.code().eq_ignore_ascii_case(text))
    }

    /// What `history::prune_audio` deletes: a day keeps today's recordings, a week the 7 days before today too, a month
    /// 31, `Off` none.
    pub fn cutoff(self) -> Cutoff {
        match self {
            Self::Off => Cutoff::All,
            Self::Day => Cutoff::DaysBack(0),
            Self::Week => Cutoff::DaysBack(7),
            Self::Month => Cutoff::DaysBack(31),
        }
    }
}

/// A period of the history settings ([`Keep`], [`AudioKeep`]) read from its name, the way `parse` reads it.
pub trait PeriodName: Copy {
    fn from_name(text: &str) -> Option<Self>;
}

impl PeriodName for Keep {
    fn from_name(text: &str) -> Option<Self> {
        Self::parse(text)
    }
}

impl PeriodName for AudioKeep {
    fn from_name(text: &str) -> Option<Self> {
        Self::parse(text)
    }
}

/// `history.keep` (a [`Keep`]) or `history.audio` (an [`AudioKeep`]) as settings.json has it: one of the periods, or a
/// value CUE does not know — a typo, a later version's name, `null` — which leaves the period unknown. While `keep` is
/// unknown the history is neither written nor cleaned up and no recording is written or deleted; while `audio` is, no
/// recording is written and only the history's own cut-off deletes any (`HistorySettings::audio_period`,
/// `audio_limit`) — until the user picks a period in the main window: a guess could delete days or recordings the user
/// keeps, or keep ones the user turned off. Such a value is written back as it was, so
/// saving the file never turns it into a period the user did not choose, and it never makes the file unreadable. A
/// missing value is the default (`Default`): a month for the history, a week for its recordings.
#[derive(Clone, Debug, PartialEq)]
pub enum Period<K = Keep> {
    Set(K),
    Unknown(serde_json::Value),
}

impl<K: Copy> Period<K> {
    /// The period, if it is known.
    pub fn known(&self) -> Option<K> {
        match self {
            Self::Set(keep) => Some(*keep),
            Self::Unknown(_) => None,
        }
    }
}

impl<K: Default + PartialEq> Period<K> {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl<K: Default> Default for Period<K> {
    fn default() -> Self {
        Self::Set(K::default())
    }
}

impl<K: Serialize> Serialize for Period<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Set(keep) => keep.serialize(serializer),
            Self::Unknown(value) => value.serialize(serializer),
        }
    }
}

impl<'de, K: PeriodName> Deserialize<'de> for Period<K> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match value.as_str().and_then(K::from_name) {
            Some(keep) => Self::Set(keep),
            None => Self::Unknown(value),
        })
    }
}

/// The history (spec 3 §2) and the recordings of its entries. A period is not written while it is the default (a
/// month, a week), like the fields of `translate`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistorySettings {
    #[serde(skip_serializing_if = "Period::is_default")]
    pub keep: Period,
    /// How long the recordings are kept, as the user chose it; what holds is `audio_period`.
    #[serde(skip_serializing_if = "Period::is_default")]
    pub audio: Period<AudioKeep>,
}

impl HistorySettings {
    /// How long recordings are kept for real: the shorter of `audio` and `keep`, so a recording never outlives its
    /// entry — `Off` while the history is off, whatever `audio` says. Otherwise `None` while either period is unknown:
    /// then no recording is written, and only `audio_limit` deletes any.
    pub fn audio_period(&self) -> Option<AudioKeep> {
        let keep = self.keep.known()?;
        if keep == Keep::Off {
            return Some(AudioKeep::Off);
        }
        let audio = self.audio.known()?;
        Some(audio_cap(keep).map_or(audio, |cap| audio.min(cap)))
    }

    /// What deletes recordings: `audio_period`, and while the recordings' own period is unknown, the history's own
    /// cut-off, so a recording still never outlives its entry — kept a year, it goes with its day. `None` while the
    /// history's period is unknown, or forever with the recordings' unknown: nothing is deleted.
    pub fn audio_limit(&self) -> Option<Cutoff> {
        match self.audio_period() {
            Some(audio) => Some(audio.cutoff()),
            None => self.keep.known()?.cutoff(),
        }
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The longest a recording may be kept while the history keeps its entries for `keep`; `None` — a period longer than
/// any recordings have.
fn audio_cap(keep: Keep) -> Option<AudioKeep> {
    match keep {
        Keep::Forever | Keep::Year => None,
        Keep::Month => Some(AudioKeep::Month),
        Keep::Week => Some(AudioKeep::Week),
        Keep::Off => Some(AudioKeep::Off),
    }
}

/// One change of a setting from the main window, as the window sends it: `{"kind": "micDevice", "value": "…"}`. The
/// core applies it to its copy of the settings with [`Settings::apply`] and writes the whole file.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum SettingChange {
    /// How long recordings are kept. An unknown period is refused, like the history's.
    #[serde(deserialize_with = "period_or_refuse")]
    HistoryAudio(AudioKeep),
    /// Unlike the file, an unknown period is refused rather than read as a month: a slip of the window must never
    /// shorten the history.
    #[serde(deserialize_with = "period_or_refuse")]
    HistoryKeep(Keep),
    /// A device name; `None` or a blank name — the Windows microphone.
    MicDevice(Option<String>),
    /// A language code CUE knows.
    TranslateTarget(String),
    UiLanguage(UiLanguage),
    /// No AI, this computer or the user's agent.
    RefineWhere(Where),
    /// The agent the cleanup goes through.
    RefineAgent(AgentId),
    /// The model of an agent; empty: the agent's default. See [`Settings::apply`] for what is refused.
    RefineAgentModel { agent: AgentId, model: String },
    /// What to paste when the AI does not answer.
    RefineFallback(Fallback),
    /// A word for the dictionary, put first; see [`Settings::apply`] for what is refused.
    DictAdd(String),
    /// A word out of the dictionary, as it is written there.
    DictRemove(String),
    /// A word from a suggestion: first in the dictionary, with how it was heard; refused like `DictAdd`, and when
    /// `heard` could not go into the request.
    DictLearn(LearnedWord),
    /// "Learn from me" on or off.
    Learn(bool),
    /// A habit on or off (docs/specs/2026-11-04-habits.md).
    HabitOn { id: String, on: bool },
    /// A habit forgotten from the main window or its card's "Undo": its forms are never learned again.
    HabitForget(String),
    /// "Forget all": every habit, the counts and the declined forms.
    HabitsForget,
}

fn period_or_refuse<'de, D: Deserializer<'de>, K: PeriodName>(deserializer: D) -> Result<K, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    value.as_str().and_then(K::from_name).ok_or_else(|| serde::de::Error::custom("unknown history period"))
}

/// Like `AudioSettings`: a log shows only the length of a device name, and a language code only when CUE knows it.
impl std::fmt::Debug for SettingChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HistoryAudio(keep) => f.debug_tuple("HistoryAudio").field(keep).finish(),
            Self::HistoryKeep(keep) => f.debug_tuple("HistoryKeep").field(keep).finish(),
            Self::MicDevice(device) => {
                f.debug_struct("MicDevice").field("chars", &device.as_ref().map(|d| d.chars().count())).finish()
            }
            Self::TranslateTarget(code) => f.debug_tuple("TranslateTarget").field(&translate::known(code)).finish(),
            Self::UiLanguage(language) => f.debug_tuple("UiLanguage").field(language).finish(),
            Self::RefineWhere(w) => f.debug_tuple("RefineWhere").field(w).finish(),
            Self::RefineAgent(agent) => f.debug_tuple("RefineAgent").field(agent).finish(),
            Self::RefineAgentModel { agent, model } => f.debug_struct("RefineAgentModel").field("agent", agent).field("model", model).finish(),
            Self::RefineFallback(fb) => f.debug_tuple("RefineFallback").field(fb).finish(),
            // A word of the dictionary may be a name: only its length.
            Self::DictAdd(word) => f.debug_struct("DictAdd").field("chars", &word.chars().count()).finish(),
            Self::DictRemove(word) => f.debug_struct("DictRemove").field("chars", &word.chars().count()).finish(),
            Self::DictLearn(l) => f
                .debug_struct("DictLearn")
                .field("chars", &l.word.chars().count())
                .field("heard_chars", &l.heard.chars().count())
                .finish(),
            Self::Learn(on) => f.debug_tuple("Learn").field(on).finish(),
            // The ids are hashes, never text.
            Self::HabitOn { id, on } => f.debug_struct("HabitOn").field("id", id).field("on", on).finish(),
            Self::HabitForget(id) => f.debug_tuple("HabitForget").field(id).finish(),
            Self::HabitsForget => f.write_str("HabitsForget"),
        }
    }
}

impl Settings {
    /// Applies one change from the main window; `true` if a value changed (then the core writes the file). A value
    /// the settings would not hold is refused (`false`, nothing changes): a microphone name longer than
    /// `MAX_DEVICE_CHARS` characters, a translation language CUE does not know. Values are kept the way reading the
    /// file keeps them: a microphone name trimmed, a blank one as the Windows microphone, the default translation
    /// language (`en`) as no value. A period set here makes an unknown one known. A dictionary word is trimmed and put
    /// first; refused are one the refine request could not take (`refine::is_term`), one already there in any case,
    /// and any once the dictionary holds `refine::MAX_TERMS` words. Removing takes the word as it is written.
    /// `DictLearn` puts a word in the same way, with how it was heard (refused too when that is no term); `Learn` sets
    /// "Learn from me", whose default (on) is written as no value.
    /// `HabitOn` switches a habit, `HabitForget` forgets one and declines its forms, `HabitsForget` forgets them all;
    /// each is refused when it changes nothing.
    pub fn apply(&mut self, change: SettingChange) -> bool {
        match change {
            SettingChange::HistoryAudio(keep) => set_if_changed(&mut self.history.audio, Period::Set(keep)),
            SettingChange::HistoryKeep(keep) => set_if_changed(&mut self.history.keep, Period::Set(keep)),
            SettingChange::MicDevice(device) => {
                let device = device.as_deref().map(str::trim).filter(|d| !d.is_empty());
                if device.is_some_and(|d| d.chars().count() > MAX_DEVICE_CHARS) {
                    return false;
                }
                set_if_changed(&mut self.audio.device, device.map(str::to_owned))
            }
            SettingChange::TranslateTarget(code) => {
                let Some(code) = translate::known(&code) else { return false };
                set_if_changed(&mut self.translate.target, (code != DEFAULT_TARGET).then(|| code.to_owned()))
            }
            SettingChange::UiLanguage(language) => set_if_changed(&mut self.ui.language, language),
            SettingChange::RefineWhere(w) => set_if_changed(&mut self.refine.where_, w),
            SettingChange::RefineAgent(agent) => set_if_changed(&mut self.refine.agent, Some(agent)),
            SettingChange::RefineAgentModel { agent, model } => {
                let model = model.trim();
                // Codex and Grok have a list CUE knows; OpenCode lists its own, so any short name goes.
                let known = agent.models();
                let listed = if known.is_empty() { model.chars().count() <= MAX_MODEL_CHARS } else { known.iter().any(|(id, _)| *id == model) };
                if !model.is_empty() && !listed {
                    return false;
                }
                let default = known.first().map(|(id, _)| *id);
                let chosen = (!model.is_empty() && Some(model) != default).then(|| model.to_owned());
                set_entry(&mut self.refine.models, agent.id(), chosen)
            }
            SettingChange::RefineFallback(fb) => set_if_changed(&mut self.refine.fallback, fb),
            SettingChange::DictAdd(word) => self.dictionary.put_first(word.trim()),
            SettingChange::DictLearn(l) => {
                let (word, heard) = (l.word.trim(), l.heard.trim());
                if !refine::is_term(heard) || !self.dictionary.put_first(word) {
                    return false;
                }
                self.dictionary.learned.retain(|x| x.word.to_lowercase() != word.to_lowercase());
                self.dictionary.learned.insert(0, LearnedWord { word: word.to_owned(), heard: heard.to_owned() });
                true
            }
            SettingChange::DictRemove(word) => {
                let d = &mut self.dictionary;
                let (terms, learned) = (d.terms.len(), d.learned.len());
                d.terms.retain(|t| *t != word);
                d.learned.retain(|l| l.word != word);
                d.terms.len() != terms || d.learned.len() != learned
            }
            SettingChange::Learn(on) => set_if_changed(&mut self.learn.enabled, (!on).then_some(false)),
            SettingChange::HabitOn { id, on } => self.habits.switch(&id, on),
            SettingChange::HabitForget(id) => self.habits.forget(&id),
            SettingChange::HabitsForget => self.habits.forget_all(),
        }
    }
}

/// Puts `value` into `slot`; whether it was different.
fn set_if_changed<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        return false;
    }
    *slot = value;
    true
}

/// Sets or removes `key` in `map`; `true` if the map changed.
fn set_entry(map: &mut BTreeMap<String, String>, key: &str, value: Option<String>) -> bool {
    match value {
        Some(v) if map.get(key) != Some(&v) => {
            map.insert(key.to_owned(), v);
            true
        }
        None => map.remove(key).is_some(),
        _ => false,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub stt: SttSettings,
    pub refine: RefineSettings,
    pub hold_hints_left: u8,
    pub bench: BenchSettings,
    /// Not written while it is the default (`auto`): files of users who never set it stay as they were.
    #[serde(skip_serializing_if = "UiSettings::is_default")]
    pub ui: UiSettings,
    /// Where the Drop sits. Not written while it is the default (`bottom`), like `ui`.
    #[serde(skip_serializing_if = "DropSettings::is_default")]
    pub drop: DropSettings,
    /// Not written while the list is empty, like `ui`.
    #[serde(skip_serializing_if = "DictionarySettings::is_default")]
    pub dictionary: DictionarySettings,
    /// "Learn from me". Not written while it is on, like `translate`.
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "LearnSettings::is_default")]
    pub learn: LearnSettings,
    /// Habits (docs/specs/2026-11-04-habits.md): counts, habits and declined forms. Not written while empty.
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "Habits::is_default")]
    pub habits: Habits,
    /// Not written while every field is the default, like `ui`.
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "TranslateSettings::is_default")]
    pub translate: TranslateSettings,
    /// Not written while every field is the default, like `translate`.
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "AskSettings::is_default")]
    pub ask: AskSettings,
    /// The pinned microphone. Not written while it follows Windows, like `translate`.
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "AudioSettings::is_default")]
    pub audio: AudioSettings,
    /// How long the history keeps its days and their recordings. Not written while it is the default, like `translate`.
    #[serde(deserialize_with = "section_or_default", skip_serializing_if = "HistorySettings::is_default")]
    pub history: HistorySettings,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("reading/writing settings: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings.json is corrupt: {0}")]
    Parse(#[from] serde_json::Error),
}

impl Default for RefineSettings {
    fn default() -> Self {
        Self {
            where_: Where::Agent,
            local: String::new(),
            agent: None,
            models: BTreeMap::new(),
            fallback: Fallback::Raw,
            context_chars: 500,
            min_ratio: 0.35,
            max_ratio: 1.25,
            min_overlap: 0.7,
            legacy_custom: None,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stt: SttSettings::default(),
            refine: RefineSettings::default(),
            hold_hints_left: 2,
            bench: BenchSettings::default(),
            ui: UiSettings::default(),
            drop: DropSettings::default(),
            dictionary: DictionarySettings::default(),
            learn: LearnSettings::default(),
            habits: Habits::default(),
            translate: TranslateSettings::default(),
            ask: AskSettings::default(),
            audio: AudioSettings::default(),
            history: HistorySettings::default(),
        }
    }
}

impl UiSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl DropSettings {
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl DictionarySettings {
    /// A word first in `terms`: refused when empty, not one line, too long, there in any case, or the list is full.
    fn put_first(&mut self, word: &str) -> bool {
        let taken = self.terms.iter().any(|t| t.trim().to_lowercase() == word.to_lowercase());
        if !refine::is_term(word) || taken || self.terms.len() >= refine::MAX_TERMS {
            return false;
        }
        self.terms.insert(0, word.to_owned());
        true
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// No file: default settings. A corrupt file: an error (the user's file is never overwritten).
/// A UTF-8 byte order mark (some Windows editors add one) is skipped.
pub fn load(path: &Path) -> Result<Settings, SettingsError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let json = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            Ok(serde_json::from_slice(json)?)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.into()),
    }
}

/// The write is atomic: first a temporary file next to the target, then a rename over it.
/// A failure mid-write never leaves a truncated settings.json.
pub fn save(settings: &Settings, path: &Path) -> Result<(), SettingsError> {
    let json = serde_json::to_string_pretty(settings)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = temp_path(path)?;
    let written = write_synced(&tmp, json.as_bytes()).and_then(|()| std::fs::rename(&tmp, path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// A temporary name in the same directory (a rename across volumes is not atomic); unique within the process.
fn temp_path(path: &Path) -> std::io::Result<PathBuf> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let Some(name) = path.file_name() else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "the settings path has no file name"));
    };
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(".{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    Ok(path.with_file_name(tmp_name))
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    // Data must be on disk before the rename: otherwise a power failure can leave an empty file.
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cue-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn defaults_are_as_documented() {
        let s = Settings::default();
        assert_eq!(s.stt.engine, SttEngineKind::Parakeet);
        assert_eq!((s.refine.where_, s.refine.agent, s.refine.fallback), (Where::Agent, None, Fallback::Raw));
        assert_eq!(s.refine.context_chars, 500);
        assert_eq!((s.refine.min_ratio, s.refine.max_ratio, s.refine.min_overlap), (0.35, 1.25, 0.7));
        assert_eq!(s.hold_hints_left, 2);
        assert!(!s.bench.save_recordings);
        assert_eq!(s.ui.language, UiLanguage::Auto);
        assert_eq!(s.drop.position, DropPosition::Bottom);
        assert!(s.dictionary.terms.is_empty());
        assert_eq!(s.ask, AskSettings::default());
    }

    #[test]
    fn missing_file_gives_defaults() {
        assert_eq!(load(&tmp("missing.json")).unwrap(), Settings::default());
    }

    #[test]
    fn partial_file_keeps_other_defaults_and_ignores_unknown() {
        let p = tmp("partial.json");
        std::fs::write(&p, r#"{"stt":{"soniox_model":"stt-rt-v6","future_stt":true},"refine":{"model":"claude-sonnet-5-5"},"future":1}"#).unwrap();
        let s = load(&p).unwrap();
        assert_eq!(s.stt.engine, SttEngineKind::Parakeet);
        assert_eq!((s.refine.where_, s.refine.agent, s.refine.fallback), (Where::Agent, None, Fallback::Raw));
        assert_eq!(s.refine.context_chars, 500);
        std::fs::remove_file(p).unwrap();
    }

    /// A file from earlier versions: fields of removed engines (Soniox, ElevenLabs: names, models, hints, prices) and
    /// values the user changed. It loads without error, the engine is Parakeet, everything else is kept; after a save
    /// the old fields are gone.
    #[test]
    fn legacy_file_with_removed_engines_loads_as_parakeet_and_keeps_other_values() {
        let dir = tmp("legacy");
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &p,
            r#"{
              "stt": {
                "engine": "elevenlabs",
                "soniox_model": "stt-rt-v6",
                "soniox_async_model": "stt-async-v5",
                "elevenlabs_model": "scribe_v2_realtime",
                "language_hints": ["ru"]
              },
              "refine": { "model": "claude-sonnet-5-5", "context_chars": 300 },
              "hold_hints_left": 0,
              "bench": { "save_recordings": true },
              "pricing": {
                "soniox_rt_per_hour": 0.15,
                "soniox_async_per_hour": 0.10,
                "elevenlabs_rt_per_hour": 0.39,
                "claude": [
                  { "model": "claude-haiku-5-5", "input_per_mtok": 0.1, "output_per_mtok": 0.5,
                    "cache_read_per_mtok": 0.01, "cache_write_per_mtok": 0.125 }
                ]
              }
            }"#,
        )
        .unwrap();
        let s = load(&p).expect("a file from earlier versions must load");

        let mut expected = Settings::default();
        expected.refine.context_chars = 300;
        expected.hold_hints_left = 0;
        expected.bench.save_recordings = true;
        assert_eq!(s.stt.engine, SttEngineKind::Parakeet);
        assert_eq!(s, expected);

        save(&s, &p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("elevenlabs") && !text.contains("soniox"), "old fields are still in the file: {text}");
        assert!(!text.contains("pricing") && !text.contains("per_mtok"), "prices are no setting: {text}");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["stt"]["engine"], "parakeet");
        assert_eq!(load(&p).unwrap(), expected);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The chosen speech model: Ultra when the file names none; whichever id it names is kept as written (the runtime
    /// reads an id it cannot find as Ultra).
    #[test]
    fn the_speech_model_is_ultra_until_another_is_written() {
        assert_eq!(Settings::default().stt.model, crate::model::DEFAULT_ID);
        let p = tmp("stt-model.json");
        std::fs::write(&p, r#"{"stt":{"engine":"parakeet"}}"#).unwrap();
        let mut s = load(&p).unwrap();
        assert_eq!(s.stt.model, crate::model::DEFAULT_ID);
        s.stt.model = "hf--someone--parakeet-tdt".into();
        save(&s, &p).unwrap();
        assert_eq!(load(&p).unwrap().stt.model, "hf--someone--parakeet-tdt");
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn unknown_engine_name_falls_back_to_parakeet() {
        let p = tmp("unknown-engine.json");
        std::fs::write(&p, r#"{"stt":{"engine":"something-new"},"hold_hints_left":1}"#).unwrap();
        let s = load(&p).expect("an unknown engine is no reason for a load error");
        assert_eq!(s.stt.engine, SttEngineKind::Parakeet);
        assert_eq!(s.hold_hints_left, 1);
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn engine_is_written_and_read_as_parakeet() {
        assert_eq!(serde_json::to_string(&SttEngineKind::Parakeet).unwrap(), r#""parakeet""#);
        assert_eq!(serde_json::from_str::<SttEngineKind>(r#""parakeet""#).unwrap(), SttEngineKind::Parakeet);
        assert_eq!(serde_json::from_str::<SttEngineKind>(r#""soniox""#).unwrap(), SttEngineKind::Parakeet);
    }

    #[test]
    fn broken_file_is_an_error() {
        let p = tmp("broken.json");
        std::fs::write(&p, "{ not json").unwrap();
        assert!(matches!(load(&p), Err(SettingsError::Parse(_))));
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn save_then_load_roundtrip_creates_dirs() {
        let dir = tmp("dir");
        let p = dir.join("nested").join("settings.json");
        let s = Settings { hold_hints_left: 0, ..Settings::default() };
        save(&s, &p).unwrap();
        assert_eq!(load(&p).unwrap(), s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn save_is_atomic_replaces_file_and_leaves_no_temp() {
        let dir = tmp("atomic");
        let p = dir.join("settings.json");
        // A first write, then an overwrite on top of the existing file.
        save(&Settings::default(), &p).unwrap();
        let mut s = Settings::default();
        s.refine.context_chars = 300;
        s.bench.save_recordings = true;
        save(&s, &p).unwrap();
        assert_eq!(load(&p).unwrap(), s);
        // Only settings.json is in the directory: the temporary file was renamed, not left beside it.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.json"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_rename_returns_io_error_and_removes_temp() {
        let dir = tmp("rename-fail");
        // A directory sits where settings.json should be: a rename over it will fail.
        std::fs::create_dir_all(dir.join("settings.json")).unwrap();
        let err = save(&Settings::default(), &dir.join("settings.json")).unwrap_err();
        assert!(matches!(err, SettingsError::Io(_)), "expected an Io error, got {err:?}");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.json"], "the temporary file was not removed");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Prices of an earlier version (CUE kept them for usage accounting), even ones the user changed: the file loads as
    /// it did, and a save leaves them out.
    #[test]
    fn prices_of_an_earlier_version_are_read_past_and_never_written() {
        let dir = tmp("old-pricing");
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, r#"{"hold_hints_left":1,"pricing":{"web_search_usd":0.5,"claude":[]}}"#).unwrap();
        let s = load(&p).unwrap();
        assert_eq!(s, Settings { hold_hints_left: 1, ..Settings::default() });
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(json.get("pricing").is_none(), "{json}");
        assert!(json.get("stt").is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn utf8_bom_is_skipped() {
        let p = tmp("bom.json");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"hold_hints_left":1}"#);
        std::fs::write(&p, bytes).unwrap();
        let s = load(&p).unwrap();
        assert_eq!(s.hold_hints_left, 1);
        assert_eq!(s.stt, SttSettings::default());
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn invalid_utf8_is_a_parse_error() {
        let p = tmp("badutf8.json");
        std::fs::write(&p, b"{\"refine\":{\"style\":\"\xff\xfe\"}}").unwrap();
        assert!(matches!(load(&p), Err(SettingsError::Parse(_))));
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn ui_language_reads_en_ru_and_auto() {
        for (text, expected) in [("en", UiLanguage::En), ("ru", UiLanguage::Ru), ("auto", UiLanguage::Auto)] {
            let s: Settings = serde_json::from_str(&format!(r#"{{"ui":{{"language":"{text}"}}}}"#)).unwrap();
            assert_eq!(s.ui.language, expected, "{text}");
        }
    }

    #[test]
    fn ui_language_ignores_case_and_surrounding_spaces() {
        for (text, expected) in
            [("RU", UiLanguage::Ru), (" ru ", UiLanguage::Ru), ("En", UiLanguage::En), ("\tEN\n", UiLanguage::En), (" Auto ", UiLanguage::Auto)]
        {
            let json = serde_json::json!({ "ui": { "language": text } }).to_string();
            let s: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(s.ui.language, expected, "{text:?}");
        }
    }

    fn read_refine(json: &str) -> RefineSettings {
        serde_json::from_str::<Settings>(json).unwrap_or_else(|e| panic!("{json}: {e}")).refine
    }

    const OLD_RECOMMENDED: &str = "llm--unsloth--qwen3-4b-instruct-2507-gguf--q4_k_m";

    #[test]
    fn runs_local_keeps_its_model() {
        let s = read_refine(r#"{"refine":{"runs":"local","local":" llm--qwen--qwen3-4b-gguf--q4_k_m ","context_chars":100}}"#);
        assert_eq!((s.where_, s.local.as_str(), s.context_chars), (Where::Local, "llm--qwen--qwen3-4b-gguf--q4_k_m", 100));
        assert_eq!(s.agent, None);
        // A local server of an earlier version ran on this computer too.
        for id in ["ollama", " LMStudio "] {
            let s = read_refine(&format!(r#"{{"refine":{{"connection":"{id}","local":"llm--a--b--q4_k_m"}}}}"#));
            assert_eq!((s.where_, s.local.as_str()), (Where::Local, "llm--a--b--q4_k_m"), "{id}");
        }
    }

    #[test]
    fn runs_local_with_empty_local_gets_old_recommended_id() {
        // The id is the one `llm::id_for` builds for the model earlier versions recommended.
        assert_eq!(crate::llm::id_for("unsloth/Qwen3-4B-Instruct-2507-GGUF", "Q4_K_M"), OLD_RECOMMENDED);
        for json in [r#"{"refine":{"runs":"local"}}"#, r#"{"refine":{"runs":" LOCAL ","local":"  "}}"#] {
            let s = read_refine(json);
            assert_eq!((s.where_, s.local.as_str()), (Where::Local, OLD_RECOMMENDED), "{json}");
        }
    }

    #[test]
    fn runs_cloud_becomes_agent_unresolved() {
        for json in [
            r#"{"refine":{"runs":"cloud","connection":"openai","models":{"openai":"gpt-6.1-terra"}}}"#,
            r#"{"refine":{"runs":"other"}}"#,
            r#"{"refine":{"runs":1}}"#,
            r#"{"refine":{"connection":"anthropic"}}"#,
            r#"{"refine":{}}"#,
        ] {
            let s = read_refine(json);
            assert_eq!((s.where_, s.agent), (Where::Agent, None), "{json}");
            assert!(s.models.is_empty(), "a cloud connection's model is dropped: {json}");
        }
        assert_eq!(Settings::default().refine.where_, Where::Agent);
        let s = read_refine(r#"{"refine":{"runs":"cloud","agent":" Codex ","models":{"codex":"gpt-6-sol","openai":"x"}}}"#);
        assert_eq!((s.where_, s.agent), (Where::Agent, Some(AgentId::Codex)));
        assert_eq!(s.models.get("codex").map(String::as_str), Some("gpt-6-sol"));
        assert_eq!(s.models.len(), 1);
    }

    #[test]
    fn custom_style_text_is_legacy_custom() {
        let s = read_refine(r#"{"refine":{"style":" Custom ","custom":"  пиши коротко \n"}}"#);
        assert_eq!(s.legacy_custom(), Some("пиши коротко"));
        for json in [
            r#"{"refine":{"style":"custom","custom":" \n "}}"#,
            r#"{"refine":{"style":"default","custom":"пиши коротко"}}"#,
            r#"{"refine":{"style":"bold","custom":"пиши коротко"}}"#,
            r#"{"refine":{"custom":"пиши коротко"}}"#,
            r#"{"refine":{"style":"custom"}}"#,
        ] {
            assert_eq!(read_refine(json).legacy_custom(), None, "{json}");
        }
        assert_eq!(Settings::default().refine.legacy_custom(), None);
    }

    /// Whatever an earlier version left, none of its fields is written again, and the file reads back the same.
    #[test]
    fn old_fields_are_never_written_back() {
        let dir = tmp("old-fields");
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        let old = r#"{"refine":{"runs":"cloud","style":"custom","custom":"пиши коротко","connection":"openai",
            "model":"claude-sonnet-5-5","level":"low","addresses":{"ollama":"http://x"},"models":{"openai":"gpt-6.1-terra"},
            "context_chars":300},"ask":{"model":"claude-opus-5-5"}}"#;
        std::fs::write(&p, old).unwrap();
        let mut s = load(&p).unwrap();
        // Until the instructions file exists the old text stays in the file; the runtime clears it afterwards.
        s.refine.legacy_custom = None;
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        for absent in ["runs", "style", "custom", "connection", "model", "level", "addresses", "models", "legacy_custom"] {
            assert!(json["refine"].get(absent).is_none(), "refine.{absent}: {json}");
        }
        assert!(json.get("ask").is_none_or(|a| a.get("model").is_none()), "ask.model: {json}");
        assert_eq!(json["refine"]["where"], "agent");
        assert_eq!(json["refine"]["fallback"], "raw");
        assert_eq!(json["refine"]["context_chars"], 300);
        // Written once, read again: the same (the old text was for the instructions file, not for the next start).
        let again = load(&p).unwrap();
        assert_eq!((again.refine.where_, again.refine.context_chars, again.refine.legacy_custom()), (Where::Agent, 300, None));
        s.refine.legacy_custom = None;
        assert_eq!(again, s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// One realistic file of an earlier version: what matters survives, and once the instructions file has the text
    /// the saved file holds no old field.
    #[test]
    fn a_realistic_old_file_moves_over() {
        let dir = tmp("realistic-old");
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        let old = r#"{"refine":{"runs":"local","local":"llm--qwen--qwen3-4b-gguf--q4_k_m","style":"custom","custom":" пиши коротко ",
            "connection":"openai","models":{"openai":"gpt-6.1-terra"},"model":"x","context_chars":300},
            "ask":{"model":"claude-opus-5-5","max_searches":4}}"#;
        std::fs::write(&p, old).unwrap();
        let mut s = load(&p).unwrap();
        assert_eq!((s.refine.where_, s.refine.local.as_str()), (Where::Local, "llm--qwen--qwen3-4b-gguf--q4_k_m"));
        assert_eq!((s.refine.legacy_custom(), s.refine.context_chars, s.ask.max_searches()), (Some("пиши коротко"), 300, 4));
        // Saved before the instructions file exists: the text is still in the file and is found on the next start.
        save(&s, &p).unwrap();
        assert_eq!(load(&p).unwrap().refine.legacy_custom(), Some("пиши коротко"));
        s.refine.legacy_custom = None;
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        for gone in ["runs", "style", "custom", "connection", "model", "models"] {
            assert!(json["refine"].get(gone).is_none(), "refine.{gone}: {json}");
        }
        assert_eq!(json["ask"], serde_json::json!({ "max_searches": 4 }));
        assert_eq!(json["refine"]["where"], "local");
        let again = load(&p).unwrap();
        assert_eq!((again.refine.where_, again.refine.local.as_str(), again.refine.legacy_custom()), (Where::Local, "llm--qwen--qwen3-4b-gguf--q4_k_m", None));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migrated_agent_models_are_validated_like_apply() {
        let s = read_refine(r#"{"refine":{"models":{"codex":"garbage","opencode":"x/y","grok":"grok-4.6"}}}"#);
        assert_eq!(s.models.get("opencode").map(String::as_str), Some("x/y"));
        assert_eq!(s.models.len(), 1, "an unlisted model and a default are dropped: {:?}", s.models);
        let s = read_refine(r#"{"refine":{"models":{"codex":"gpt-6-sol","grok":"grok-4.5","opencode":"  "}}}"#);
        assert_eq!(s.models.get("codex").map(String::as_str), Some("gpt-6-sol"));
        assert_eq!(s.models.get("grok").map(String::as_str), Some("grok-4.5"));
        assert_eq!(s.models.len(), 2);
        let long = "m".repeat(201);
        assert!(read_refine(&format!(r#"{{"refine":{{"models":{{"opencode":"{long}"}}}}}}"#)).models.is_empty());
    }

    #[test]
    fn unknown_where_reads_as_default_agent() {
        for (value, expected) in [
            (r#""off""#, Where::Off),
            (r#"" LOCAL ""#, Where::Local),
            (r#""agent""#, Where::Agent),
            (r#""cloud""#, Where::Agent),
            (r#""""#, Where::Agent),
            ("1", Where::Agent),
            ("null", Where::Agent),
        ] {
            let s = read_refine(&format!(r#"{{"refine":{{"where":{value},"runs":"local","context_chars":100}}}}"#));
            assert_eq!((s.where_, s.context_chars), (expected, 100), "{value}");
        }
        assert_eq!(serde_json::to_value(RefineSettings::default()).unwrap()["where"], "agent");
    }

    #[test]
    fn fallback_is_raw_unless_local() {
        for (value, expected) in [(r#""local""#, Fallback::Local), (r#"" Local ""#, Fallback::Local), (r#""raw""#, Fallback::Raw), (r#""x""#, Fallback::Raw), ("3", Fallback::Raw)] {
            let s = read_refine(&format!(r#"{{"refine":{{"where":"agent","fallback":{value}}}}}"#));
            assert_eq!(s.fallback, expected, "{value}");
        }
    }

    #[test]
    fn unknown_agent_reads_as_none() {
        for value in [r#""claude""#, r#""gemini""#, "1", "null", r#""""#] {
            assert_eq!(read_refine(&format!(r#"{{"refine":{{"agent":{value}}}}}"#)).agent, None, "{value}");
        }
        assert_eq!(read_refine(r#"{"refine":{"agent":"grok"}}"#).agent, Some(AgentId::Grok));
    }

    #[test]
    fn local_usage_model_names_the_local_model() {
        let s = RefineSettings { where_: Where::Local, local: "llm--qwen--qwen3-4b-gguf--q4_k_m".into(), ..RefineSettings::default() };
        assert_eq!(s.usage_model(), "local:llm--qwen--qwen3-4b-gguf--q4_k_m");
        let s = RefineSettings { where_: Where::Local, ..RefineSettings::default() };
        assert_eq!(s.usage_model(), "off", "nothing chosen: no local cleanup");
        assert_eq!(RefineSettings::default().usage_model(), "off", "no agent resolved yet");
    }

    #[test]
    fn agent_usage_model_names_the_agent_and_its_model() {
        let mut s = RefineSettings { where_: Where::Agent, agent: Some(AgentId::Codex), ..RefineSettings::default() };
        assert_eq!(s.usage_model(), "agent:codex:gpt-6-luna", "Codex's default");
        s.models.insert("codex".into(), "gpt-6-sol".into());
        assert_eq!(s.usage_model(), "agent:codex:gpt-6-sol");
        s.agent = Some(AgentId::OpenCode);
        assert_eq!(s.usage_model(), "agent:opencode:auto");
        assert_eq!(s.agent_model(AgentId::OpenCode), None);
        s.models.insert("opencode".into(), "anthropic/claude-haiku-5-5".into());
        assert_eq!(s.agent_model(AgentId::OpenCode), Some("anthropic/claude-haiku-5-5"));
        s.where_ = Where::Off;
        assert_eq!(s.usage_model(), "off");
    }

    #[test]
    fn apply_switches_where() {
        let mut s = Settings::default();
        assert!(s.apply(SettingChange::RefineWhere(Where::Local)));
        assert_eq!(s.refine.where_, Where::Local);
        assert!(!s.apply(SettingChange::RefineWhere(Where::Local)), "the same value is no change");
        assert!(s.apply(SettingChange::RefineWhere(Where::Off)));
    }

    /// `level` and `addresses` of earlier versions are read past and never written again; empty choices are not written.
    #[test]
    fn old_refine_fields_are_read_past() {
        let s = read_refine(r#"{"refine":{"model":"claude-sonnet-5-5","level":"low","style":"bold","context_chars":300}}"#);
        assert_eq!(s.context_chars, 300);
        let written = serde_json::to_value(&s).unwrap();
        for absent in ["model", "level", "models", "addresses", "custom", "agent", "local"] {
            assert!(written.get(absent).is_none(), "{absent}: {written}");
        }
    }

    /// Unknown values mean `auto`, and the rest of the file still loads.
    #[test]
    fn unknown_ui_language_reads_as_auto_and_keeps_the_rest() {
        for value in [r#""fr""#, r#""r u""#, r#""ru-RU""#, r#""""#, r#"" ""#, "5", "null", "true", "[]", "{}"] {
            let text = format!(r#"{{"ui":{{"language":{value},"future":1}},"hold_hints_left":0}}"#);
            let s: Settings = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{value}: {e}"));
            assert_eq!(s.ui.language, UiLanguage::Auto, "{value}");
            assert_eq!(s.hold_hints_left, 0, "{value}");
        }
    }

    #[test]
    fn file_without_ui_block_loads_as_auto() {
        let p = tmp("no-ui.json");
        std::fs::write(&p, r#"{"stt":{"soniox_model":"stt-rt-v6"},"hold_hints_left":1}"#).unwrap();
        let s = load(&p).unwrap();
        assert_eq!(s.ui, UiSettings::default());
        assert_eq!(s.ui.language, UiLanguage::Auto);
        assert_eq!(s.hold_hints_left, 1);
        std::fs::remove_file(p).unwrap();
    }

    /// The default (`auto`, also when the file says so explicitly) is not written back.
    #[test]
    fn default_ui_language_is_not_written() {
        let dir = tmp("default-ui");
        let p = dir.join("settings.json");
        save(&Settings::default(), &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(json.get("ui").is_none(), "the default language ended up in the file: {json}");

        std::fs::write(&p, r#"{"ui":{"language":"auto"},"hold_hints_left":2}"#).unwrap();
        let s = load(&p).unwrap();
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(json.get("ui").is_none(), "an explicit auto was written back: {json}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chosen_ui_language_is_written_and_survives_roundtrip() {
        let dir = tmp("chosen-ui");
        let p = dir.join("settings.json");
        for (language, code) in [(UiLanguage::Ru, "ru"), (UiLanguage::En, "en")] {
            let mut s = Settings::default();
            s.ui.language = language;
            save(&s, &p).unwrap();
            let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            assert_eq!(json["ui"], serde_json::json!({ "language": code }));
            assert_eq!(load(&p).unwrap(), s);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dictionary_terms_are_read_as_written() {
        let s: Settings = serde_json::from_str(r#"{"dictionary":{"terms":["Soniox"," GitHub ","Soniox",""],"future":1}}"#).unwrap();
        // Trimming, repeats and empty entries are handled on use (`refine::terms_prompt`), not on load.
        assert_eq!(s.dictionary.terms, ["Soniox", " GitHub ", "Soniox", ""]);
        assert_eq!(s.stt, SttSettings::default());
    }

    #[test]
    fn file_without_dictionary_has_no_terms() {
        for text in [r#"{"hold_hints_left":1}"#, r#"{"dictionary":{}}"#, r#"{"dictionary":{"terms":[]}}"#] {
            let s: Settings = serde_json::from_str(text).unwrap();
            assert!(s.dictionary.terms.is_empty(), "{text}");
        }
    }

    /// A typo in the list never makes the whole file unreadable: anything but a list reads as an empty list,
    /// items that are not strings are skipped, the rest of the file still loads.
    #[test]
    fn invalid_terms_read_as_empty_or_are_skipped() {
        for value in [r#""Soniox""#, "5", "null", "true", "{}"] {
            let text = format!(r#"{{"dictionary":{{"terms":{value}}},"hold_hints_left":0}}"#);
            let s: Settings = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{value}: {e}"));
            assert!(s.dictionary.terms.is_empty(), "{value}");
            assert_eq!(s.hold_hints_left, 0, "{value}");
        }
        let s: Settings = serde_json::from_str(r#"{"dictionary":{"terms":["Soniox",5,null,["x"],{"a":1},true,"CUE"]}}"#).unwrap();
        assert_eq!(s.dictionary.terms, ["Soniox", "CUE"]);
    }

    /// Like `ui`: an empty list (also an explicit one) is not written back; a list with terms is, and survives a roundtrip.
    #[test]
    fn empty_dictionary_is_not_written_and_terms_are() {
        let dir = tmp("dictionary");
        let p = dir.join("settings.json");
        let written = |p: &Path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap();
        save(&Settings::default(), &p).unwrap();
        assert!(written(&p).get("dictionary").is_none(), "an empty dictionary ended up in the file");

        std::fs::write(&p, r#"{"dictionary":{"terms":[]},"hold_hints_left":2}"#).unwrap();
        save(&load(&p).unwrap(), &p).unwrap();
        assert!(written(&p).get("dictionary").is_none(), "an explicit empty list was written back");

        let mut s = Settings::default();
        s.dictionary.terms = vec!["Soniox".into(), " Claude Haiku".into()];
        save(&s, &p).unwrap();
        assert_eq!(written(&p)["dictionary"], serde_json::json!({ "terms": ["Soniox", " Claude Haiku"] }));
        assert_eq!(load(&p).unwrap(), s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn drop_position_reads_all_four_in_any_case() {
        for (text, expected) in [
            ("bottom", DropPosition::Bottom),
            ("top", DropPosition::Top),
            ("left", DropPosition::Left),
            ("right", DropPosition::Right),
            ("LEFT", DropPosition::Left),
            (" Right ", DropPosition::Right),
            ("\tTop\n", DropPosition::Top),
        ] {
            let json = serde_json::json!({ "drop": { "position": text } }).to_string();
            let s: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(s.drop.position, expected, "{text:?}");
        }
    }

    /// A typo in the position or in the whole block reads as the default and the rest of the file still loads.
    #[test]
    fn invalid_drop_position_reads_as_bottom_and_keeps_the_rest() {
        for drop in [
            r#"{"position":"middle"}"#,
            r#"{"position":"le ft"}"#,
            r#"{"position":""}"#,
            r#"{"position":5}"#,
            r#"{"position":null}"#,
            r#"{"position":["left"]}"#,
            r#"{"future":1}"#,
            r#"{}"#,
            r#""left""#,
            "5",
            "null",
            "[]",
        ] {
            let text = format!(r#"{{"drop":{drop},"hold_hints_left":0}}"#);
            let s: Settings = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{drop}: {e}"));
            assert_eq!(s.drop.position, DropPosition::Bottom, "{drop}");
            assert_eq!(s.hold_hints_left, 0, "{drop}");
        }
    }

    #[test]
    fn file_without_drop_block_reads_as_bottom() {
        let s: Settings = serde_json::from_str(r#"{"hold_hints_left":1}"#).unwrap();
        assert_eq!(s.drop, DropSettings::default());
        assert_eq!(s.drop.position, DropPosition::Bottom);
    }

    /// Like `ui`: the default (also an explicit `"bottom"`) is not written; another place is, and survives a roundtrip.
    #[test]
    fn drop_position_is_written_only_when_not_bottom() {
        let dir = tmp("drop-position");
        let p = dir.join("settings.json");
        let written = |p: &Path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap();
        save(&Settings::default(), &p).unwrap();
        assert!(written(&p).get("drop").is_none(), "the default position ended up in the file");

        std::fs::write(&p, r#"{"drop":{"position":"BOTTOM"},"hold_hints_left":2}"#).unwrap();
        save(&load(&p).unwrap(), &p).unwrap();
        assert!(written(&p).get("drop").is_none(), "an explicit bottom was written back");

        for position in [DropPosition::Top, DropPosition::Left, DropPosition::Right] {
            let mut s = Settings::default();
            s.drop.position = position;
            save(&s, &p).unwrap();
            assert_eq!(written(&p)["drop"], serde_json::json!({ "position": position.code() }));
            assert_eq!(load(&p).unwrap(), s);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn translate_defaults_and_reading() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.translate.target(), "en");
        assert_eq!(s.translate.speech(), None);
        let s: Settings = serde_json::from_str(r#"{"translate":{"target":" DE ","home":"ru-RU","speech":"auto"}}"#).unwrap();
        assert_eq!(s.translate.target(), "de");
        assert_eq!(s.translate.home.as_deref(), Some("ru"));
        assert_eq!(s.translate.speech(), None);
    }

    #[test]
    fn bad_translate_values_read_as_defaults() {
        for json in [r#"{"translate":"en"}"#, r#"{"translate":5}"#, r#"{"translate":{"target":"klingon","speech":7}}"#] {
            let s: Settings = serde_json::from_str(json).unwrap_or_else(|e| panic!("{json}: {e}"));
            assert_eq!(s.translate.target(), "en", "{json}");
            assert_eq!(s.translate.speech(), None, "{json}");
        }
    }

    #[test]
    fn default_translate_is_not_written() {
        let text = serde_json::to_string(&Settings::default()).unwrap();
        assert!(!text.contains("translate"), "{text}");
    }

    /// Like an explicit `ui.language` `"auto"`: an explicit default target (`en`, in any spelling) reads as the default
    /// and is not written back; the other fields are kept.
    #[test]
    fn explicit_default_target_is_not_written_back() {
        let dir = tmp("default-target");
        let p = dir.join("settings.json");
        let written = |p: &Path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        for target in ["en", " EN ", "en-US"] {
            let text = serde_json::json!({ "translate": { "target": target }, "hold_hints_left": 2 }).to_string();
            std::fs::write(&p, text).unwrap();
            let s = load(&p).unwrap();
            assert_eq!(s.translate, TranslateSettings::default(), "{target:?}");
            save(&s, &p).unwrap();
            assert!(written(&p).get("translate").is_none(), "an explicit {target:?} was written back");
        }
        std::fs::write(&p, r#"{"translate":{"target":"en","home":"ru"}}"#).unwrap();
        save(&load(&p).unwrap(), &p).unwrap();
        assert_eq!(written(&p)["translate"], serde_json::json!({ "home": "ru" }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Only the fields that were set are written, and they read back the same.
    #[test]
    fn chosen_translate_fields_are_written_and_survive_roundtrip() {
        let dir = tmp("translate");
        let p = dir.join("settings.json");
        let mut s = Settings::default();
        s.translate.target = Some("de".into());
        s.translate.speech = Some("ru".into());
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(json["translate"], serde_json::json!({ "target": "de", "speech": "ru" }));
        assert_eq!(load(&p).unwrap(), s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn ask_defaults_and_reading() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.ask.enabled() && s.ask.web_search());
        assert_eq!(s.ask.max_searches(), 2);
        let s: Settings = serde_json::from_str(
            r#"{"ask":{"enabled":false,"web_search":false,"max_searches":5}}"#,
        )
        .unwrap();
        assert!(!s.ask.enabled() && !s.ask.web_search());
        assert_eq!(s.ask.max_searches(), 5);
    }

    /// The Ask model of earlier versions is read past and never written again.
    #[test]
    fn ask_model_is_read_past() {
        let dir = tmp("ask-model");
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, r#"{"ask":{"model":"claude-sonnet-5-5","max_searches":5}}"#).unwrap();
        let s = load(&p).unwrap();
        assert_eq!(s.ask.max_searches(), 5);
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(json["ask"], serde_json::json!({ "max_searches": 5 }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A typo in `ask` reads as the default and never makes the file unreadable.
    #[test]
    fn bad_ask_values_read_as_defaults() {
        for json in [
            r#"{"ask":"off"}"#,
            r#"{"ask":5}"#,
            r#"{"ask":{"enabled":"no","web_search":0,"model":"","max_searches":0}}"#,
            r#"{"ask":{"enabled":null,"model":7,"max_searches":11}}"#,
            r#"{"ask":{"max_searches":-1}}"#,
            r#"{"ask":{"max_searches":2.5}}"#,
            r#"{"ask":{"max_searches":"3"}}"#,
        ] {
            let s: Settings = serde_json::from_str(json).unwrap_or_else(|e| panic!("{json}: {e}"));
            assert_eq!(s.ask, AskSettings::default(), "{json}");
        }
    }

    /// Like `translate.target`: a default, also one the file states, is not written back.
    #[test]
    fn default_ask_values_are_not_written_back() {
        let dir = tmp("default-ask");
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &p,
            r#"{"ask":{"enabled":true,"web_search":true,"model":"claude-haiku-5-5","max_searches":2},"hold_hints_left":2}"#,
        )
        .unwrap();
        let s = load(&p).unwrap();
        assert_eq!(s.ask, AskSettings::default());
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(json.get("ask").is_none(), "{json}");
        assert!(!serde_json::to_string(&Settings::default()).unwrap().contains("\"ask\""));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chosen_ask_fields_are_written_and_survive_roundtrip() {
        let dir = tmp("ask");
        let p = dir.join("settings.json");
        let mut s = Settings::default();
        s.ask.web_search = Some(false);
        s.ask.max_searches = Some(4);
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(json["ask"], serde_json::json!({ "web_search": false, "max_searches": 4 }));
        assert_eq!(load(&p).unwrap(), s);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn drop_position_codes_parse_back() {
        for position in DropPosition::ALL {
            assert_eq!(DropPosition::parse(position.code()), Some(position));
            assert_eq!(serde_json::to_string(&position).unwrap(), format!("\"{}\"", position.code()));
        }
        assert_eq!(DropPosition::parse("center"), None);
    }

    /// Like `translate`: the defaults (also ones the file states) are not written, so the file of a user who never
    /// opened the main window is written back exactly as it was.
    #[test]
    fn audio_and_history_defaults_are_not_written() {
        let dir = tmp("default-audio-history");
        let p = dir.join("settings.json");
        let written = |p: &Path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap();
        save(&Settings::default(), &p).unwrap();
        assert!(written(&p).get("audio").is_none(), "the default microphone ended up in the file");
        assert!(written(&p).get("history").is_none(), "the default retention ended up in the file");

        std::fs::write(&p, r#"{"audio":{"device":" "},"history":{"keep":"MONTH"},"hold_hints_left":2}"#).unwrap();
        save(&load(&p).unwrap(), &p).unwrap();
        assert!(written(&p).get("audio").is_none(), "a blank device was written back");
        assert!(written(&p).get("history").is_none(), "an explicit month was written back");

        // A file of an earlier version, with values the user changed and none of the new sections.
        let mut earlier = Settings::default();
        earlier.refine.context_chars = 300;
        earlier.hold_hints_left = 0;
        earlier.ui.language = UiLanguage::Ru;
        earlier.translate.target = Some("de".into());
        save(&earlier, &p).unwrap();
        let before = std::fs::read(&p).unwrap();
        let s = load(&p).unwrap();
        assert_eq!((s.audio.device.as_deref(), s.history.keep.known()), (None, Some(Keep::Month)));
        save(&s, &p).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), before, "the file changed on a write without a change");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn chosen_mic_and_keep_are_written_and_survive_roundtrip() {
        let dir = tmp("chosen-audio-history");
        let p = dir.join("settings.json");
        for keep in [Keep::Forever, Keep::Year, Keep::Week, Keep::Off] {
            let mut s = Settings::default();
            s.audio.device = Some("Microphone (USB Audio Device)".into());
            s.history.keep = Period::Set(keep);
            save(&s, &p).unwrap();
            let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            assert_eq!(json["audio"], serde_json::json!({ "device": "Microphone (USB Audio Device)" }));
            assert_eq!(json["history"], serde_json::json!({ "keep": keep.code() }));
            assert_eq!(load(&p).unwrap(), s);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A period CUE does not know leaves it unknown (the history is neither written nor cleaned up), not a month; a
    /// missing one — or a `history` section that is not an object, like any other section — is a month.
    #[test]
    fn history_keep_unknown_is_unknown_not_a_month() {
        for (text, expected) in
            [("forever", Keep::Forever), (" Year ", Keep::Year), ("WEEK", Keep::Week), ("off", Keep::Off), ("month", Keep::Month)]
        {
            let json = serde_json::json!({ "history": { "keep": text } }).to_string();
            let s: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(s.history.keep, Period::Set(expected), "{text:?}");
        }
        for keep in [r#""always""#, r#""for ever""#, r#""""#, "31", "null", "true", r#"["week"]"#] {
            let text = format!(r#"{{"history":{{"keep":{keep}}},"hold_hints_left":0}}"#);
            let s: Settings = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{keep}: {e}"));
            let value: serde_json::Value = serde_json::from_str(keep).unwrap();
            assert_eq!(s.history.keep, Period::Unknown(value), "{keep}");
            assert_eq!(s.history.keep.known(), None, "{keep}");
            assert_eq!(s.hold_hints_left, 0, "{keep}");
        }
        for history in [r#"{"future":1}"#, r#"{}"#, r#""week""#, "5", "null", "[]"] {
            let text = format!(r#"{{"history":{history},"hold_hints_left":0}}"#);
            let s: Settings = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{history}: {e}"));
            assert_eq!(s.history.keep.known(), Some(Keep::Month), "{history}");
        }
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().history, HistorySettings::default());
    }

    /// Saving the file (the Drop dragged, a hint shown) never turns an unknown period into one the user did not
    /// choose: it is written back as it was. A period set from the main window replaces it.
    #[test]
    fn an_unknown_period_is_written_back_as_it_was() {
        let dir = tmp("unknown-period");
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, r#"{"history":{"keep":"forver"},"hold_hints_left":2}"#).unwrap();
        let mut s = load(&p).unwrap();
        s.hold_hints_left = 1;
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(json["history"], serde_json::json!({ "keep": "forver" }));
        assert_eq!(load(&p).unwrap().history.keep.known(), None);

        assert!(s.apply(SettingChange::HistoryKeep(Keep::Year)));
        save(&s, &p).unwrap();
        assert_eq!(load(&p).unwrap().history.keep, Period::Set(Keep::Year));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Recordings are kept a week unless the user chose otherwise. Like the history's period, the default — also one
    /// the file states — is not written, and a chosen period is, beside the history's.
    #[test]
    fn history_audio_is_a_week_and_written_only_when_chosen() {
        let dir = tmp("history-audio");
        let p = dir.join("settings.json");
        let written = |p: &Path| serde_json::from_slice::<serde_json::Value>(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!(Settings::default().history.audio, Period::Set(AudioKeep::Week));
        for text in [r#"{}"#, r#"{"history":{"keep":"year"}}"#, r#"{"history":"day"}"#] {
            let s: Settings = serde_json::from_str(text).unwrap();
            assert_eq!(s.history.audio.known(), Some(AudioKeep::Week), "{text}");
        }
        for (text, expected) in
            [("day", AudioKeep::Day), (" Week ", AudioKeep::Week), ("MONTH", AudioKeep::Month), ("off", AudioKeep::Off)]
        {
            let json = serde_json::json!({ "history": { "audio": text } }).to_string();
            let s: Settings = serde_json::from_str(&json).unwrap();
            assert_eq!(s.history.audio, Period::Set(expected), "{text:?}");
        }

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, r#"{"history":{"keep":"month","audio":" WEEK "},"hold_hints_left":2}"#).unwrap();
        save(&load(&p).unwrap(), &p).unwrap();
        assert!(written(&p).get("history").is_none(), "explicit defaults were written back");

        for (keep, audio, expected) in [
            (Keep::Month, AudioKeep::Day, serde_json::json!({ "audio": "day" })),
            (Keep::Year, AudioKeep::Week, serde_json::json!({ "keep": "year" })),
            (Keep::Week, AudioKeep::Off, serde_json::json!({ "keep": "week", "audio": "off" })),
            (Keep::Forever, AudioKeep::Month, serde_json::json!({ "keep": "forever", "audio": "month" })),
        ] {
            let history = HistorySettings { keep: Period::Set(keep), audio: Period::Set(audio) };
            let s = Settings { history, ..Settings::default() };
            save(&s, &p).unwrap();
            assert_eq!(written(&p)["history"], expected);
            assert_eq!(load(&p).unwrap(), s);
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A recording period CUE does not know is unknown, like the history's: no recording is written (`audio_period`)
    /// and only the history's period deletes any (`audio_limit`), and it is written back as it was until the main
    /// window sets a period. The history's own names that recordings do not have (`forever`, `year`) are unknown too.
    #[test]
    fn history_audio_unknown_is_unknown_and_written_back() {
        for audio in [r#""fortnight""#, r#""forever""#, r#""year""#, r#""""#, "7", "null", "true", r#"["day"]"#] {
            let text = format!(r#"{{"history":{{"audio":{audio}}},"hold_hints_left":0}}"#);
            let s: Settings = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{audio}: {e}"));
            let value: serde_json::Value = serde_json::from_str(audio).unwrap();
            assert_eq!(s.history.audio, Period::Unknown(value), "{audio}");
            assert_eq!((s.history.keep.known(), s.history.audio_period()), (Some(Keep::Month), None), "{audio}");
            assert_eq!(s.history.audio_limit(), Some(Cutoff::DaysBack(31)), "{audio}: the history's month");
            assert_eq!(s.hold_hints_left, 0, "{audio}");
        }

        let dir = tmp("unknown-audio");
        let p = dir.join("settings.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&p, r#"{"history":{"keep":"week","audio":"fortnite"},"hold_hints_left":2}"#).unwrap();
        let mut s = load(&p).unwrap();
        s.hold_hints_left = 1;
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert_eq!(json["history"], serde_json::json!({ "keep": "week", "audio": "fortnite" }));
        assert_eq!(load(&p).unwrap().history.audio_period(), None);

        assert!(s.apply(SettingChange::HistoryAudio(AudioKeep::Day)));
        save(&s, &p).unwrap();
        let history = load(&p).unwrap().history;
        assert_eq!((history.audio_period(), history.audio), (Some(AudioKeep::Day), Period::Set(AudioKeep::Day)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A recording never outlives its entry: the shorter of the two periods holds, and off while the history is off
    /// whatever the recordings' period. Otherwise none while either is unknown — but while the recordings' own is, the
    /// history's still deletes them.
    #[test]
    fn audio_period_is_never_longer_than_the_history() {
        use AudioKeep::{Day, Month, Off, Week};
        for (keep, expected) in [
            (Keep::Forever, [Day, Week, Month, Off]),
            (Keep::Year, [Day, Week, Month, Off]),
            (Keep::Month, [Day, Week, Month, Off]),
            (Keep::Week, [Day, Week, Week, Off]),
            (Keep::Off, [Off, Off, Off, Off]),
        ] {
            for (audio, expected) in AudioKeep::ALL.into_iter().zip(expected) {
                let history = HistorySettings { keep: Period::Set(keep), audio: Period::Set(audio) };
                assert_eq!(history.audio_period(), Some(expected), "{keep:?} {audio:?}");
                let limit = history.audio_limit();
                assert_eq!(limit, Some(expected.cutoff()), "{keep:?} {audio:?}: a known period is the limit");
            }
        }
        let unknown = serde_json::json!("forver");
        for audio in AudioKeep::ALL {
            let history = HistorySettings { keep: Period::Unknown(unknown.clone()), audio: Period::Set(audio) };
            assert_eq!((history.audio_period(), history.audio_limit()), (None, None), "{audio:?}");
        }
        // The recordings' period unknown: the history's own cut-off, a year's too; forever deletes none.
        for (keep, period, limit) in [
            (Keep::Forever, None, None),
            (Keep::Year, None, Some(Cutoff::DaysBack(366))),
            (Keep::Month, None, Some(Cutoff::DaysBack(31))),
            (Keep::Week, None, Some(Cutoff::DaysBack(7))),
            (Keep::Off, Some(Off), Some(Cutoff::All)),
        ] {
            let history = HistorySettings { keep: Period::Set(keep), audio: Period::Unknown(unknown.clone()) };
            assert_eq!((history.audio_period(), history.audio_limit()), (period, limit), "{keep:?}");
        }
        let both = HistorySettings { keep: Period::Unknown(unknown.clone()), audio: Period::Unknown(unknown) };
        assert_eq!((both.audio_period(), both.audio_limit()), (None, None));
        assert_eq!(HistorySettings::default().audio_period(), Some(Week));
        assert_eq!(HistorySettings::default().audio_limit(), Some(Cutoff::DaysBack(7)));
        let cutoffs = AudioKeep::ALL.map(AudioKeep::cutoff);
        assert_eq!(cutoffs, [Cutoff::DaysBack(0), Cutoff::DaysBack(7), Cutoff::DaysBack(31), Cutoff::All]);
        let cutoffs = Keep::ALL.map(Keep::cutoff);
        let days = [None, Some(Cutoff::DaysBack(366)), Some(Cutoff::DaysBack(31)), Some(Cutoff::DaysBack(7))];
        assert_eq!(cutoffs, [days[0], days[1], days[2], days[3], Some(Cutoff::All)]);
    }

    #[test]
    fn audio_keep_codes_parse_back() {
        for keep in AudioKeep::ALL {
            assert_eq!(AudioKeep::parse(keep.code()), Some(keep));
            assert_eq!(serde_json::to_string(&keep).unwrap(), format!("\"{}\"", keep.code()));
        }
        for other in ["forever", "year", "days", "", "da y"] {
            assert_eq!(AudioKeep::parse(other), None, "{other:?}");
        }
    }

    #[test]
    fn audio_device_blank_or_not_a_string_reads_as_windows() {
        for audio in [
            r#"{"device":""}"#,
            r#"{"device":"  \t "}"#,
            r#"{"device":5}"#,
            r#"{"device":null}"#,
            r#"{"device":true}"#,
            r#"{"device":["USB"]}"#,
            r#"{"device":{"name":"USB"}}"#,
            r#"{"future":1}"#,
            r#"{}"#,
            r#""USB""#,
            "5",
            "null",
            "[]",
        ] {
            let text = format!(r#"{{"audio":{audio},"hold_hints_left":0}}"#);
            let s: Settings = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{audio}: {e}"));
            assert_eq!(s.audio.device, None, "{audio}");
            assert_eq!(s.hold_hints_left, 0, "{audio}");
        }
        let s: Settings = serde_json::from_str(r#"{"audio":{"device":" Microphone (USB) "}}"#).unwrap();
        assert_eq!(s.audio.device.as_deref(), Some("Microphone (USB)"));
    }

    #[test]
    fn apply_trims_mic_and_blank_means_windows() {
        let mut s = Settings::default();
        assert!(s.apply(SettingChange::MicDevice(Some("  Microphone (USB) \t".into()))));
        assert_eq!(s.audio.device.as_deref(), Some("Microphone (USB)"));
        assert!(s.apply(SettingChange::MicDevice(Some("   ".into()))));
        assert_eq!(s.audio.device, None);

        // A name longer than 200 characters is refused and the setting stays; the limit counts characters, not bytes.
        assert!(s.apply(SettingChange::MicDevice(Some("Mic".into()))));
        assert!(!s.apply(SettingChange::MicDevice(Some("m".repeat(201)))));
        assert_eq!(s.audio.device.as_deref(), Some("Mic"));
        assert!(s.apply(SettingChange::MicDevice(Some(format!(" {} ", "м".repeat(200))))));
        assert_eq!(s.audio.device, Some("м".repeat(200)));
        assert!(s.apply(SettingChange::MicDevice(None)));
        assert_eq!(s.audio.device, None);
    }

    #[test]
    fn apply_rejects_unknown_target_and_drops_the_default() {
        let mut s = Settings::default();
        assert!(!s.apply(SettingChange::TranslateTarget("klingon".into())));
        assert!(!s.apply(SettingChange::TranslateTarget(String::new())));
        assert_eq!(s.translate.target, None);
        assert!(s.apply(SettingChange::TranslateTarget(" DE ".into())));
        assert_eq!((s.translate.target.as_deref(), s.translate.target()), (Some("de"), "de"));
        assert!(!s.apply(SettingChange::TranslateTarget("xx".into())));
        assert_eq!(s.translate.target.as_deref(), Some("de"), "a refused change keeps the language");
        // The default target is no value, as when the file is read: it is not written back.
        assert!(s.apply(SettingChange::TranslateTarget("en-US".into())));
        assert_eq!((s.translate.target.as_deref(), s.translate.target()), (None, "en"));
    }

    #[test]
    fn apply_reports_no_change() {
        let mut s = Settings::default();
        for change in [
            SettingChange::HistoryAudio(AudioKeep::Week),
            SettingChange::HistoryKeep(Keep::Month),
            SettingChange::MicDevice(None),
            SettingChange::MicDevice(Some(" ".into())),
            SettingChange::TranslateTarget("en".into()),
            SettingChange::UiLanguage(UiLanguage::Auto),
            SettingChange::RefineWhere(Where::Agent),
            SettingChange::RefineAgentModel { agent: AgentId::Codex, model: " ".into() },
            SettingChange::RefineAgentModel { agent: AgentId::Codex, model: "gpt-6-luna".into() },
            SettingChange::RefineFallback(Fallback::Raw),
        ] {
            assert!(!s.apply(change.clone()), "{change:?}");
        }
        assert_eq!(s, Settings::default());

        for change in [
            SettingChange::HistoryAudio(AudioKeep::Day),
            SettingChange::HistoryKeep(Keep::Week),
            SettingChange::MicDevice(Some("Mic".into())),
            SettingChange::TranslateTarget("fr".into()),
            SettingChange::UiLanguage(UiLanguage::Ru),
            SettingChange::RefineWhere(Where::Local),
            SettingChange::RefineAgent(AgentId::Codex),
            SettingChange::RefineAgentModel { agent: AgentId::Codex, model: "gpt-6.1-terra".into() },
            SettingChange::RefineFallback(Fallback::Local),
        ] {
            assert!(s.apply(change.clone()), "{change:?}");
            assert!(!s.apply(change.clone()), "the same change twice: {change:?}");
        }
        assert!(!s.apply(SettingChange::MicDevice(Some(" Mic ".into()))));
        assert!(!s.apply(SettingChange::TranslateTarget("FR".into())));
        assert_eq!((s.history.keep.known(), s.audio.device.as_deref()), (Some(Keep::Week), Some("Mic")));
        assert_eq!(s.history.audio.known(), Some(AudioKeep::Day));
        assert_eq!((s.translate.target(), s.ui.language), ("fr", UiLanguage::Ru));
        assert_eq!((s.refine.where_, s.refine.agent, s.refine.fallback), (Where::Local, Some(AgentId::Codex), Fallback::Local));
        assert_eq!(s.refine.models.get("codex").map(String::as_str), Some("gpt-6.1-terra"));
    }

    #[test]
    fn apply_refuses_what_the_cleanup_could_not_use() {
        let mut s = Settings::default();
        assert!(!s.apply(SettingChange::RefineAgentModel { agent: AgentId::Codex, model: "gpt-4".into() }), "a model CUE does not list");
        assert!(!s.apply(SettingChange::RefineAgentModel { agent: AgentId::OpenCode, model: "m".repeat(201) }), "too long");
        assert_eq!(s, Settings::default());
        // A model chosen and then the agent's default again: nothing is kept.
        assert!(s.apply(SettingChange::RefineAgentModel { agent: AgentId::Codex, model: "gpt-6-sol".into() }));
        assert!(s.apply(SettingChange::RefineAgentModel { agent: AgentId::Codex, model: "gpt-6-luna".into() }));
        assert!(s.refine.models.is_empty());
        // OpenCode lists its own models: any name goes, trimmed; empty is "Быстрая · авто".
        assert!(s.apply(SettingChange::RefineAgentModel { agent: AgentId::OpenCode, model: " anthropic/claude ".into() }));
        assert_eq!(s.refine.models.get("opencode").map(String::as_str), Some("anthropic/claude"));
        assert!(s.apply(SettingChange::RefineAgentModel { agent: AgentId::OpenCode, model: String::new() }));
        assert!(s.refine.models.is_empty());
    }

    /// The window sends `{"kind": …, "value": …}`. An unknown retention is refused rather than read as a month, and an
    /// unknown recording period as well — the history's own names among them.
    #[test]
    fn setting_change_reads_the_window_json() {
        let read = |json: &str| serde_json::from_str::<SettingChange>(json);
        assert_eq!(read(r#"{"kind":"historyKeep","value":"week"}"#).unwrap(), SettingChange::HistoryKeep(Keep::Week));
        let audio = SettingChange::HistoryAudio;
        assert_eq!(read(r#"{"kind":"historyAudio","value":"day"}"#).unwrap(), audio(AudioKeep::Day));
        assert_eq!(read(r#"{"kind":"historyAudio","value":"off"}"#).unwrap(), audio(AudioKeep::Off));
        assert_eq!(read(r#"{"kind":"micDevice","value":"Mic"}"#).unwrap(), SettingChange::MicDevice(Some("Mic".into())));
        assert_eq!(read(r#"{"kind":"micDevice","value":null}"#).unwrap(), SettingChange::MicDevice(None));
        assert_eq!(read(r#"{"kind":"translateTarget","value":"de"}"#).unwrap(), SettingChange::TranslateTarget("de".into()));
        assert_eq!(read(r#"{"kind":"uiLanguage","value":"ru"}"#).unwrap(), SettingChange::UiLanguage(UiLanguage::Ru));
        assert_eq!(read(r#"{"kind":"refineWhere","value":"off"}"#).unwrap(), SettingChange::RefineWhere(Where::Off));
        assert_eq!(read(r#"{"kind":"refineAgent","value":"codex"}"#).unwrap(), SettingChange::RefineAgent(AgentId::Codex));
        assert!(read(r#"{"kind":"refineAgent","value":"claude"}"#).is_err(), "Claude Code is never offered");
        assert_eq!(read(r#"{"kind":"refineFallback","value":"local"}"#).unwrap(), SettingChange::RefineFallback(Fallback::Local));
        let m = r#"{"kind":"refineAgentModel","value":{"agent":"codex","model":"gpt-6-sol"}}"#;
        assert_eq!(read(m).unwrap(), SettingChange::RefineAgentModel { agent: AgentId::Codex, model: "gpt-6-sol".into() });
        assert_eq!(read(r#"{"kind":"learn","value":false}"#).unwrap(), SettingChange::Learn(false));
        let word = LearnedWord { word: "GitHub".into(), heard: "гитхаб".into() };
        assert_eq!(read(r#"{"kind":"dictLearn","value":{"word":"GitHub","heard":"гитхаб"}}"#).unwrap(), SettingChange::DictLearn(word));
        for gone in ["refineStyle", "refineCustom", "refineRuns", "refineConnection", "refineModel"] {
            assert!(read(&format!(r#"{{"kind":"{gone}","value":"x"}}"#)).is_err(), "{gone} is gone");
        }
        assert!(read(r#"{"kind":"refineAddress","value":"http://localhost:1/v1"}"#).is_err(), "local servers are gone");
        for json in [
            r#"{"kind":"historyKeep","value":"always"}"#,
            r#"{"kind":"historyKeep","value":7}"#,
            r#"{"kind":"historyKeep"}"#,
            r#"{"kind":"historyAudio","value":"forever"}"#,
            r#"{"kind":"historyAudio","value":"year"}"#,
            r#"{"kind":"historyAudio","value":"fortnight"}"#,
            r#"{"kind":"historyAudio","value":1}"#,
            r#"{"kind":"historyAudio","value":null}"#,
            r#"{"kind":"historyAudio"}"#,
            r#"{"kind":"sounds","value":true}"#,
            r#"{"value":"de"}"#,
        ] {
            assert!(read(json).is_err(), "{json}");
        }
        let change = serde_json::to_value(SettingChange::HistoryKeep(Keep::Forever)).unwrap();
        assert_eq!(change, serde_json::json!({ "kind": "historyKeep", "value": "forever" }));
        let change = serde_json::to_value(SettingChange::HistoryAudio(AudioKeep::Month)).unwrap();
        assert_eq!(change, serde_json::json!({ "kind": "historyAudio", "value": "month" }));
        assert_eq!(format!("{:?}", SettingChange::HistoryAudio(AudioKeep::Day)), "HistoryAudio(Day)");
    }

    /// The device name is the user's: neither the settings nor a change show it in a log.
    #[test]
    fn debug_shows_no_device_name() {
        let mut s = Settings::default();
        s.audio.device = Some("Headset of Jane".into());
        let change = SettingChange::MicDevice(Some("Headset of Jane".into()));
        for text in [format!("{s:?}"), format!("{:?}", s.audio), format!("{change:?}")] {
            assert!(!text.contains("Jane"), "{text}");
            assert!(text.contains("Some(15)"), "the length is shown: {text}");
        }
        let unknown = SettingChange::TranslateTarget("my secret".into());
        assert!(!format!("{unknown:?}").contains("secret"));
        assert!(format!("{:?}", SettingChange::TranslateTarget("de".into())).contains("de"));
        let learn = SettingChange::DictLearn(learned("Jane Doe", "Джейн Доу"));
        for change in [SettingChange::DictAdd("Jane Doe".into()), SettingChange::DictRemove("Jane Doe".into()), learn] {
            assert!(!format!("{change:?}").contains("Jane"), "a dictionary word may be a name");
        }
    }

    /// The window's dictionary: a word goes first, trimmed; a repeat in any case, an empty or too long one, one with a
    /// line break and any past the limit are refused; removing takes the word as written.
    #[test]
    fn dictionary_words_are_added_first_and_removed() {
        let mut s = Settings::default();
        assert!(s.apply(SettingChange::DictAdd("  Kubernetes ".into())));
        assert!(s.apply(SettingChange::DictAdd("Soniox".into())));
        assert_eq!(s.dictionary.terms, ["Soniox", "Kubernetes"]);
        for refused in ["kubernetes", "SONIOX", "", "   ", "a\nb", "a\u{2028}b"] {
            assert!(!s.apply(SettingChange::DictAdd(refused.into())), "{refused:?}");
        }
        assert!(!s.apply(SettingChange::DictAdd("x".repeat(refine::MAX_TERM_CHARS + 1))));
        assert!(s.apply(SettingChange::DictAdd("я".repeat(refine::MAX_TERM_CHARS))), "characters, not bytes");
        assert!(!s.apply(SettingChange::DictRemove("soniox".into())), "removing takes the word as written");
        assert!(s.apply(SettingChange::DictRemove("Soniox".into())));
        assert_eq!(s.dictionary.terms.len(), 2);
        s.dictionary.terms = (0..refine::MAX_TERMS).map(|i| format!("w{i}")).collect();
        assert!(!s.apply(SettingChange::DictAdd("one more".into())), "the dictionary is full");
        let change: SettingChange = serde_json::from_value(serde_json::json!({ "kind": "dictAdd", "value": "GitHub" })).unwrap();
        assert_eq!(change, SettingChange::DictAdd("GitHub".into()));
    }

    fn learned(word: &str, heard: &str) -> LearnedWord {
        LearnedWord { word: word.into(), heard: heard.into() }
    }

    /// A word from a suggestion goes into both lists, first; it is refused like a typed one, and when what was heard
    /// cannot go into the request; removing the word takes it out of both.
    #[test]
    fn a_learned_word_joins_the_dictionary_with_how_it_was_heard() {
        let mut s = Settings::default();
        assert!(s.apply(SettingChange::DictAdd("Soniox".into())));
        assert!(s.apply(SettingChange::DictLearn(learned(" Marisol ", " Марисоль "))));
        assert_eq!(s.dictionary.terms, ["Marisol", "Soniox"]);
        assert_eq!(s.dictionary.learned, [learned("Marisol", "Марисоль")]);
        assert!(!s.apply(SettingChange::DictLearn(learned("marisol", "марисоль"))), "already there");
        assert!(!s.apply(SettingChange::DictLearn(learned("GitHub", "a\nb"))), "heard on two lines");
        assert!(!s.apply(SettingChange::DictLearn(learned("", "пусто"))));
        assert!(s.apply(SettingChange::DictRemove("Marisol".into())));
        assert!(s.dictionary.learned.is_empty() && s.dictionary.terms == ["Soniox"]);
        let change: SettingChange =
            serde_json::from_value(serde_json::json!({ "kind": "dictLearn", "value": { "word": "GitHub", "heard": "гитхаб" } })).unwrap();
        assert_eq!(change, SettingChange::DictLearn(learned("GitHub", "гитхаб")));
        let change: SettingChange = serde_json::from_value(serde_json::json!({ "kind": "learn", "value": false })).unwrap();
        assert_eq!(change, SettingChange::Learn(false));
        let change = SettingChange::DictLearn(learned("Marisol", "Марисоль"));
        let shown = format!("{change:?}");
        assert!(!shown.contains("Marisol") && !shown.contains("Марисоль"), "a learned word may be a name: {shown}");
        let mut s = Settings::default();
        s.dictionary.learned = vec![learned("Marisol", "Марисоль")];
        for shown in [format!("{:?}", s.dictionary.learned[0]), format!("{:?}", s.dictionary)] {
            assert!(!shown.contains("Marisol") && !shown.contains("Марисоль"), "only lengths: {shown}");
            assert!(shown.contains("word_chars: 7") && shown.contains("heard_chars: 8"), "{shown}");
        }
    }

    /// The habits keep their own section, written only when there is something in it; the window changes one at a time.
    #[test]
    fn habits_are_kept_in_their_section_and_changed_one_at_a_time() {
        let mut s = Settings::default();
        // The secret, as the runtime sets it from the key store: never written.
        s.habits.secret = crate::habits::Secret::from_bytes([7; crate::habits::SECRET_BYTES]);
        assert!(serde_json::to_value(&s).unwrap().get("habits").is_none(), "nothing written while there are none");
        let d = crate::habits::Draft { forms: vec![crate::habits::Form::Dash], app: "Word".into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        let id = s.habits.adopt(d, None, "en").id;
        assert_eq!(serde_json::to_value(&s).unwrap()["habits"]["list"][0]["after"], "a — b");
        assert!(s.apply(SettingChange::HabitOn { id: id.clone(), on: false }));
        assert!(!s.apply(SettingChange::HabitOn { id: id.clone(), on: false }), "no change");
        assert!(!s.habits.list[0].on);
        assert!(s.apply(SettingChange::HabitForget(id.clone())));
        assert_eq!(s.habits.declined, [crate::habits::Form::Dash.hash(&s.habits.secret)]);
        assert!(!s.apply(SettingChange::HabitForget(id)), "already gone");
        assert!(s.apply(SettingChange::HabitsForget));
        assert!(!s.apply(SettingChange::HabitsForget), "nothing left");
        let read = |json: serde_json::Value| serde_json::from_value::<SettingChange>(json).unwrap();
        assert_eq!(read(serde_json::json!({ "kind": "habitOn", "value": { "id": "h1", "on": true } })), SettingChange::HabitOn { id: "h1".into(), on: true });
        assert_eq!(read(serde_json::json!({ "kind": "habitForget", "value": "h1" })), SettingChange::HabitForget("h1".into()));
        assert_eq!(read(serde_json::json!({ "kind": "habitsForget" })), SettingChange::HabitsForget);
        let s: Settings = serde_json::from_str(r#"{"habits":"nonsense"}"#).unwrap();
        assert!(s.habits.is_default());
    }

    /// A habit's example and rule are the user's words: neither the settings nor a change show them in a log.
    #[test]
    fn debug_shows_no_habit_text() {
        let mut s = Settings::default();
        let d = crate::habits::Draft {
            forms: vec![crate::habits::Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() }],
            app: "Почта".into(),
            before: "Здравствуйте, Анна!".into(),
            after: "Добрый день, Анна!".into(),
            n: 3,
        };
        let id = s.habits.adopt(d, None, "ru").id;
        let shown = format!("{s:?} {:?} {:?}", SettingChange::HabitOn { id: id.clone(), on: false }, SettingChange::HabitForget(id));
        for word in ["Анна", "добрый", "здравствуйте", "Почта"] {
            assert!(!shown.contains(word), "{word} in a log");
        }
        assert_eq!(format!("{:?}", SettingChange::HabitsForget), "HabitsForget");
    }

    #[test]
    fn learning_is_on_by_default_and_only_off_is_written() {
        let mut s = Settings::default();
        assert!(s.learn.enabled());
        assert!(s.apply(SettingChange::Learn(false)));
        assert!(!s.learn.enabled());
        assert_eq!(serde_json::to_value(&s).unwrap()["learn"], serde_json::json!({ "enabled": false }));
        assert!(!s.apply(SettingChange::Learn(false)), "no change");
        assert!(s.apply(SettingChange::Learn(true)));
        assert!(serde_json::to_value(&s).unwrap().get("learn").is_none(), "the default is not written");
        for odd in [r#"{"learn":5}"#, r#"{"learn":{"enabled":"no"}}"#, r#"{"learn":{"enabled":null}}"#] {
            let s: Settings = serde_json::from_str(odd).unwrap();
            assert!(s.learn.enabled(), "{odd}");
        }
    }

    #[test]
    fn learned_words_read_leniently_and_an_empty_list_is_not_written() {
        let s: Settings = serde_json::from_str(
            r#"{"dictionary":{"terms":["Marisol"],"learned":[{"word":"Marisol","heard":"Марисоль"},{"word":1},"x"]}}"#,
        )
        .unwrap();
        assert_eq!(s.dictionary.learned, [learned("Marisol", "Марисоль")]);
        let s: Settings = serde_json::from_str(r#"{"dictionary":{"terms":["A"],"learned":7}}"#).unwrap();
        assert!(s.dictionary.learned.is_empty());
        let mut s = Settings::default();
        s.dictionary.terms = vec!["A".into()];
        assert!(serde_json::to_value(&s).unwrap()["dictionary"].get("learned").is_none());
    }
}

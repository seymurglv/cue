//! settings.json: engine and models, refine thresholds, the hint counter, bench, interface language, where the Drop
//! sits, the user's dictionary, the translation pair, "Ask", the pinned microphone, how long the history keeps its days
//! and their recordings, prices for usage accounting. The main window changes a setting one at a time
//! (`SettingChange`).
use crate::translate;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Transcription engine. There is only one now; the type stays for the `stt.engine` field in settings.json and the
/// `stt_engine` column in usage.csv. Any other name in the file (including ones from earlier versions) reads as Soniox.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SttEngineKind {
    #[default]
    #[serde(other)]
    Soniox,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SttSettings {
    pub engine: SttEngineKind,
    pub soniox_model: String,
    pub soniox_async_model: String,
    pub language_hints: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RefineSettings {
    pub model: String,
    /// How many characters before the caret to give the model.
    pub context_chars: usize,
    /// Sanity check thresholds.
    pub min_ratio: f32,
    pub max_ratio: f32,
    pub min_overlap: f32,
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
/// unusable entries); Soniox does not: it bills context tokens on every dictation.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DictionarySettings {
    #[serde(deserialize_with = "strings_or_nothing")]
    pub terms: Vec<String>,
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
    /// The speech language, passed to Soniox as the only hint; `auto` (or nothing) — Soniox detects it.
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

/// A language code Soniox knows, normalized ("RU" → "ru"); anything else (`"auto"`, an unknown code, a number, `null`)
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

/// "Ask" (spec 2 §9). Every field is optional: a missing or invalid value reads as the default, and a default — also one
/// the file states — is never written back, like `translate`.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AskSettings {
    /// "Ask" as a whole (a paid feature later); on by default.
    #[serde(deserialize_with = "off_or_nothing", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Web search in answers ($0.01 a search); on by default.
    #[serde(deserialize_with = "off_or_nothing", skip_serializing_if = "Option::is_none")]
    pub web_search: Option<bool>,
    #[serde(deserialize_with = "ask_model_or_nothing", skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(deserialize_with = "searches_or_nothing", skip_serializing_if = "Option::is_none")]
    pub max_searches: Option<u32>,
}

const DEFAULT_ASK_MODEL: &str = "claude-haiku-5-5";
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

    pub fn model(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_ASK_MODEL)
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

/// A model name, trimmed; empty, not a string, or the default reads as nothing.
fn ask_model_or_nothing<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value.as_str().map(str::trim).filter(|m| !m.is_empty() && *m != DEFAULT_ASK_MODEL).map(str::to_owned))
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
/// unknown the history is neither written nor cleaned up, and while either is unknown no recording is written or
/// deleted (`HistorySettings::audio_period`), until the user picks a period in the main window: a guess could delete
/// days or recordings the user keeps, or keep ones the user turned off. Such a value is written back as it was, so
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
    /// entry — `Off` while the history is off. `None` while either period is unknown: then no recording is written or
    /// deleted.
    pub fn audio_period(&self) -> Option<AudioKeep> {
        let (keep, audio) = (self.keep.known()?, self.audio.known()?);
        let longest = match keep {
            Keep::Forever | Keep::Year => return Some(audio),
            Keep::Month => AudioKeep::Month,
            Keep::Week => AudioKeep::Week,
            Keep::Off => AudioKeep::Off,
        };
        Some(audio.min(longest))
    }

    fn is_default(&self) -> bool {
        *self == Self::default()
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
    /// A language code Soniox knows.
    TranslateTarget(String),
    UiLanguage(UiLanguage),
}

fn period_or_refuse<'de, D: Deserializer<'de>, K: PeriodName>(deserializer: D) -> Result<K, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    value.as_str().and_then(K::from_name).ok_or_else(|| serde::de::Error::custom("unknown history period"))
}

/// Like `AudioSettings`: a log shows only the length of a device name, and a language code only when Soniox knows it.
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
        }
    }
}

impl Settings {
    /// Applies one change from the main window; `true` if a value changed (then the core writes the file). A value
    /// the settings would not hold is refused (`false`, nothing changes): a microphone name longer than
    /// `MAX_DEVICE_CHARS` characters, a translation language Soniox does not know. Values are kept the way reading the
    /// file keeps them: a microphone name trimmed, a blank one as the Windows microphone, the default translation
    /// language (`en`) as no value. A period set here makes an unknown one known.
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelPrice {
    pub model: String,
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_read_per_mtok: f64,
    pub cache_write_per_mtok: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pricing {
    pub soniox_rt_per_hour: f64,
    pub soniox_async_per_hour: f64,
    pub claude: Vec<ModelPrice>,
    /// One server-side web search of "Ask" (spec 2 §10).
    pub web_search_usd: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub stt: SttSettings,
    pub refine: RefineSettings,
    pub hold_hints_left: u8,
    pub bench: BenchSettings,
    /// Not written while it is the default (`auto`), like `pricing`: files of users who never set it stay as they were.
    #[serde(skip_serializing_if = "UiSettings::is_default")]
    pub ui: UiSettings,
    /// Where the Drop sits. Not written while it is the default (`bottom`), like `ui`.
    #[serde(skip_serializing_if = "DropSettings::is_default")]
    pub drop: DropSettings,
    /// Not written while the list is empty, like `ui`.
    #[serde(skip_serializing_if = "DictionarySettings::is_default")]
    pub dictionary: DictionarySettings,
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
    /// Default prices are not written to the file, so a price fix in the code also reaches existing installs.
    /// If the user changed even one price, the whole block is written.
    #[serde(skip_serializing_if = "Pricing::is_default")]
    pub pricing: Pricing,
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("reading/writing settings: {0}")]
    Io(#[from] std::io::Error),
    #[error("settings.json is corrupt: {0}")]
    Parse(#[from] serde_json::Error),
}

impl Default for SttSettings {
    fn default() -> Self {
        Self {
            engine: SttEngineKind::Soniox,
            soniox_model: "stt-rt-v5".into(),
            soniox_async_model: "stt-async-v5".into(),
            language_hints: vec!["ru".into(), "en".into()],
        }
    }
}

impl Default for RefineSettings {
    fn default() -> Self {
        Self { model: "claude-haiku-5-5".into(), context_chars: 500, min_ratio: 0.35, max_ratio: 1.25, min_overlap: 0.7 }
    }
}

fn price(model: &str, input: f64, output: f64, cache_read: f64) -> ModelPrice {
    // A write to the 5-minute cache costs 1.25 × the input price.
    ModelPrice { model: model.into(), input_per_mtok: input, output_per_mtok: output, cache_read_per_mtok: cache_read, cache_write_per_mtok: input * 1.25 }
}

impl Default for Pricing {
    fn default() -> Self {
        Self {
            soniox_rt_per_hour: 0.12,
            soniox_async_per_hour: 0.10,
            claude: vec![
                price("claude-haiku-5-5", 0.10, 0.50, 0.01),
                price("claude-sonnet-5-5", 2.0, 10.0, 0.10),
                price("claude-opus-5-5", 4.0, 20.0, 0.20),
            ],
            web_search_usd: 0.01,
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
            translate: TranslateSettings::default(),
            ask: AskSettings::default(),
            audio: AudioSettings::default(),
            history: HistorySettings::default(),
            pricing: Pricing::default(),
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
    fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl Pricing {
    pub fn claude_price(&self, model: &str) -> Option<&ModelPrice> {
        self.claude.iter().find(|p| p.model == model)
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
        assert_eq!(s.stt.engine, SttEngineKind::Soniox);
        assert_eq!(s.stt.soniox_model, "stt-rt-v5");
        assert_eq!(s.stt.soniox_async_model, "stt-async-v5");
        assert_eq!(s.stt.language_hints, vec!["ru", "en"]);
        assert_eq!(s.refine.model, "claude-haiku-5-5");
        assert_eq!(s.refine.context_chars, 500);
        assert_eq!((s.refine.min_ratio, s.refine.max_ratio, s.refine.min_overlap), (0.35, 1.25, 0.7));
        assert_eq!(s.hold_hints_left, 2);
        assert!(!s.bench.save_recordings);
        assert_eq!(s.ui.language, UiLanguage::Auto);
        assert_eq!(s.drop.position, DropPosition::Bottom);
        assert!(s.dictionary.terms.is_empty());
        assert_eq!(s.pricing.soniox_rt_per_hour, 0.12);
        let haiku = s.pricing.claude_price("claude-haiku-5-5").unwrap();
        assert_eq!((haiku.input_per_mtok, haiku.output_per_mtok), (0.10, 0.50));
        assert!(s.pricing.claude_price("claude-sonnet-5-5").is_some());
        assert!(s.pricing.claude_price("claude-opus-5-5").is_some());
        // Cache reads for Sonnet 5.5 and Opus 5.5 cost 0.05 × the input price (Anthropic pricing page, 2026-10-26).
        assert_eq!(s.pricing.claude_price("claude-sonnet-5-5").unwrap().cache_read_per_mtok, 0.10);
        assert_eq!(s.pricing.claude_price("claude-opus-5-5").unwrap().cache_read_per_mtok, 0.20);
        assert_eq!(s.pricing.web_search_usd, 0.01);
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
        assert_eq!(s.stt.engine, SttEngineKind::Soniox);
        assert_eq!(s.stt.soniox_model, "stt-rt-v6");
        assert_eq!(s.stt.soniox_async_model, "stt-async-v5");
        assert_eq!(s.refine.model, "claude-sonnet-5-5");
        assert_eq!(s.refine.context_chars, 500);
        std::fs::remove_file(p).unwrap();
    }

    /// A file from earlier versions: fields of a removed engine (name, model, price) and values the user changed.
    /// It loads without error, the engine is Soniox, everything else is kept; after a save the old fields are gone.
    #[test]
    fn legacy_file_with_removed_engine_loads_as_soniox_and_keeps_other_values() {
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
        expected.stt.soniox_model = "stt-rt-v6".into();
        expected.stt.language_hints = vec!["ru".into()];
        expected.refine.model = "claude-sonnet-5-5".into();
        expected.refine.context_chars = 300;
        expected.hold_hints_left = 0;
        expected.bench.save_recordings = true;
        expected.pricing.soniox_rt_per_hour = 0.15;
        expected.pricing.claude = vec![price("claude-haiku-5-5", 0.10, 0.50, 0.01)];
        assert_eq!(s.stt.engine, SttEngineKind::Soniox);
        assert_eq!(s, expected);

        save(&s, &p).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("elevenlabs"), "old fields are still in the file: {text}");
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["stt"]["engine"], "soniox");
        assert_eq!(load(&p).unwrap(), expected);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_engine_name_falls_back_to_soniox() {
        let p = tmp("unknown-engine.json");
        std::fs::write(&p, r#"{"stt":{"engine":"something-new"},"hold_hints_left":1}"#).unwrap();
        let s = load(&p).expect("an unknown engine is no reason for a load error");
        assert_eq!(s.stt.engine, SttEngineKind::Soniox);
        assert_eq!(s.hold_hints_left, 1);
        std::fs::remove_file(p).unwrap();
    }

    #[test]
    fn engine_is_written_and_read_as_soniox() {
        assert_eq!(serde_json::to_string(&SttEngineKind::Soniox).unwrap(), r#""soniox""#);
        assert_eq!(serde_json::from_str::<SttEngineKind>(r#""soniox""#).unwrap(), SttEngineKind::Soniox);
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
        s.stt.soniox_model = "stt-rt-v6".into();
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

    #[test]
    fn default_pricing_is_not_written_and_loads_back_as_default() {
        let dir = tmp("default-pricing");
        let p = dir.join("settings.json");
        save(&Settings::default(), &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(json.get("pricing").is_none(), "default prices ended up in the file: {json}");
        assert!(json.get("stt").is_some());
        assert_eq!(load(&p).unwrap().pricing, Pricing::default());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn edited_pricing_is_written_and_survives_roundtrip() {
        let dir = tmp("edited-pricing");
        let p = dir.join("settings.json");
        let mut s = Settings::default();
        s.pricing.soniox_rt_per_hour = 0.5;
        save(&s, &p).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
        assert!(json.get("pricing").is_some());
        assert_eq!(load(&p).unwrap(), s);
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
        std::fs::write(&p, b"{\"stt\":{\"soniox_model\":\"\xff\xfe\"}}").unwrap();
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

    /// Like `pricing`: the default (`auto`, also when the file says so explicitly) is not written back.
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
        assert_eq!((s.ask.model(), s.ask.max_searches()), ("claude-haiku-5-5", 2));
        let s: Settings = serde_json::from_str(
            r#"{"ask":{"enabled":false,"web_search":false,"model":" claude-sonnet-5-5 ","max_searches":5}}"#,
        )
        .unwrap();
        assert!(!s.ask.enabled() && !s.ask.web_search());
        assert_eq!((s.ask.model(), s.ask.max_searches()), ("claude-sonnet-5-5", 5));
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
        earlier.stt.language_hints = vec!["ru".into()];
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

    /// A recording period CUE does not know is unknown, like the history's: no recording is written or deleted
    /// (`audio_period`), and it is written back as it was until the main window sets a period. The history's own
    /// names that recordings do not have (`forever`, `year`) are unknown too.
    #[test]
    fn history_audio_unknown_is_unknown_and_written_back() {
        for audio in [r#""fortnight""#, r#""forever""#, r#""year""#, r#""""#, "7", "null", "true", r#"["day"]"#] {
            let text = format!(r#"{{"history":{{"audio":{audio}}},"hold_hints_left":0}}"#);
            let s: Settings = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{audio}: {e}"));
            let value: serde_json::Value = serde_json::from_str(audio).unwrap();
            assert_eq!(s.history.audio, Period::Unknown(value), "{audio}");
            assert_eq!((s.history.keep.known(), s.history.audio_period()), (Some(Keep::Month), None), "{audio}");
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

    /// A recording never outlives its entry: the shorter of the two periods holds, and none while either is unknown.
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
            }
        }
        let unknown = serde_json::json!("forver");
        for audio in AudioKeep::ALL {
            let history = HistorySettings { keep: Period::Unknown(unknown.clone()), audio: Period::Set(audio) };
            assert_eq!(history.audio_period(), None, "{audio:?}");
        }
        for keep in Keep::ALL {
            let history = HistorySettings { keep: Period::Set(keep), audio: Period::Unknown(unknown.clone()) };
            assert_eq!(history.audio_period(), None, "{keep:?}");
        }
        assert_eq!(HistorySettings::default().audio_period(), Some(Week));
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
        ] {
            assert!(s.apply(change.clone()), "{change:?}");
            assert!(!s.apply(change.clone()), "the same change twice: {change:?}");
        }
        assert!(!s.apply(SettingChange::MicDevice(Some(" Mic ".into()))));
        assert!(!s.apply(SettingChange::TranslateTarget("FR".into())));
        assert_eq!((s.history.keep.known(), s.audio.device.as_deref()), (Some(Keep::Week), Some("Mic")));
        assert_eq!(s.history.audio.known(), Some(AudioKeep::Day));
        assert_eq!((s.translate.target(), s.ui.language), ("fr", UiLanguage::Ru));
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
    }
}

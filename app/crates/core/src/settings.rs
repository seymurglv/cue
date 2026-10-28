//! settings.json: engine and models, refine thresholds, the hint counter, bench, interface language, the user's
//! dictionary, prices for usage accounting.
use serde::{Deserialize, Deserializer, Serialize};
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
    /// Not written while the list is empty, like `ui`.
    #[serde(skip_serializing_if = "DictionarySettings::is_default")]
    pub dictionary: DictionarySettings,
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
            dictionary: DictionarySettings::default(),
            pricing: Pricing::default(),
        }
    }
}

impl UiSettings {
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
        assert!(s.dictionary.terms.is_empty());
        assert_eq!(s.pricing.soniox_rt_per_hour, 0.12);
        let haiku = s.pricing.claude_price("claude-haiku-5-5").unwrap();
        assert_eq!((haiku.input_per_mtok, haiku.output_per_mtok), (0.10, 0.50));
        assert!(s.pricing.claude_price("claude-sonnet-5-5").is_some());
        assert!(s.pricing.claude_price("claude-opus-5-5").is_some());
        // Cache reads for Sonnet 5.5 and Opus 5.5 cost 0.05 × the input price (Anthropic pricing page, 2026-10-26).
        assert_eq!(s.pricing.claude_price("claude-sonnet-5-5").unwrap().cache_read_per_mtok, 0.10);
        assert_eq!(s.pricing.claude_price("claude-opus-5-5").unwrap().cache_read_per_mtok, 0.20);
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
}

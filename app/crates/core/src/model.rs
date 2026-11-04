//! The speech models CUE can download (specs "Local speech recognition" and "Choosing the speech model"): which files,
//! from which commit of which repository, with their sizes and SHA-256; the short list pinned here; whether a Hugging
//! Face repository is one CUE can run; the manifest a found model keeps in its folder; and the states the windows show.
use std::path::Path;

use serde::{Deserialize, Serialize};

/// One file of a model: where it is in the repository, its name in the model's folder, its size and SHA-256.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

/// A model: a Hugging Face repository at a fixed commit (never `main`: the files must be the ones checked here), and
/// the folder it goes into under `models` (`id`). `languages` — ISO codes of the languages it recognizes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub commit: String,
    pub files: Vec<ModelFile>,
    pub languages: Vec<String>,
    pub license: String,
}

/// The model CUE uses until another is chosen.
pub const DEFAULT_ID: &str = "parakeet-ultra-int8";
/// A found model's manifest, in its folder.
pub const MANIFEST_FILE: &str = "manifest.json";
/// The three files CUE runs, as the onnx-asr exports name them.
pub const ENCODER: &str = "encoder-model.int8.onnx";
pub const DECODER: &str = "decoder_joint-model.int8.onnx";
pub const VOCAB: &str = "vocab.txt";
/// The prefix of a found model's id: `hf--<owner>--<repository>`.
const FOUND: &str = "hf--";

/// The 25 European languages of Parakeet v3 and of the models post-trained from it.
const EUROPEAN: [&str; 25] = [
    "en", "es", "fr", "de", "bg", "hr", "cs", "da", "nl", "et", "fi", "el", "hu", "it", "lv", "lt", "mt", "pl", "pt",
    "ro", "sk", "sl", "sv", "ru", "uk",
];

/// A model of the short list: its three files at `folder` in the repository (`vocab.txt` at its root).
fn pinned(
    id: &str,
    name: &str,
    repo: &str,
    commit: &str,
    folder: &str,
    [vocab, decoder, encoder]: [(u64, &str); 3],
    languages: &[&str],
) -> Manifest {
    let file = |path: String, name: &str, (size, sha256): (u64, &str)| ModelFile {
        path,
        name: name.to_owned(),
        size,
        sha256: sha256.to_owned(),
    };
    let at = |name: &str| if folder.is_empty() { name.to_owned() } else { format!("{folder}/{name}") };
    Manifest {
        id: id.to_owned(),
        name: name.to_owned(),
        repo: repo.to_owned(),
        commit: commit.to_owned(),
        files: vec![file(VOCAB.to_owned(), VOCAB, vocab), file(at(DECODER), DECODER, decoder), file(at(ENCODER), ENCODER, encoder)],
        languages: languages.iter().map(|&l| l.to_owned()).collect(),
        license: "cc-by-4.0".to_owned(),
    }
}

/// Parakeet Ultra (Moondream's post-trained NVIDIA Parakeet TDT 0.6B v3), int8 ONNX export (CC-BY-4.0): the default.
pub fn ultra() -> Manifest {
    pinned(
        DEFAULT_ID,
        "Parakeet Ultra",
        "Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx",
        "b27f93c2238aa35e138996c005a62f1e8430f801",
        "int8",
        [
            (93_939, "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d"),
            (18_203_490, "667ffde2242a560923cba6bc7f69ef03d736f8a36ca9588f2e6f9fa818cf2fc1"),
            (649_431_593, "714047d70be4b612dce02e0ab2f9e0d6552f78adc421f408d154070796bc2dcf"),
        ],
        &EUROPEAN,
    )
}

/// The short list, the default first: Ultra, NVIDIA's own v3 and the English-only v2 (onnx-asr exports, CC-BY-4.0).
pub fn catalog() -> Vec<Manifest> {
    vec![
        ultra(),
        pinned(
            "parakeet-v3-int8",
            "Parakeet v3",
            "istupakov/parakeet-tdt-0.6b-v3-onnx",
            "8f23f0c03c8761650bdb5b40aaf3e40d2c15f1ce",
            "",
            [
                (93_939, "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d"),
                (18_202_004, "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70"),
                (652_183_999, "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09"),
            ],
            &EUROPEAN,
        ),
        pinned(
            "parakeet-v2-int8",
            "Parakeet v2",
            "istupakov/parakeet-tdt-0.6b-v2-onnx",
            "0bbb45a3365852604aef28b538a8f066f4ccaa85",
            "",
            [
                (9_384, "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d"),
                (8_998_286, "a449f49acd68979d418651dd2dcb737cc0f1bf0225e009e29ee326354edbf7d3"),
                (652_184_014, "3e0581fda6ab843888b51e56d7ee78b6d5bc3237ec113af1f732d1d5286aa155"),
            ],
            &["en"],
        ),
    ]
}

impl Manifest {
    pub fn url(&self, f: &ModelFile) -> String {
        format!("https://huggingface.co/{}/resolve/{}/{}", self.repo, self.commit, f.path)
    }

    /// Bytes of every file together.
    pub fn total(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// A manifest CUE can trust to name files in its folder: an id that is a folder name, a commit that is not a
    /// branch, plain file names, SHA-256 in hex.
    pub(crate) fn sound(&self) -> bool {
        let plain = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) && !s.starts_with('.');
        let hex = |s: &str, n: usize| s.len() == n && s.chars().all(|c| c.is_ascii_hexdigit());
        plain(&self.id)
            && hex(&self.commit, 40)
            && !self.files.is_empty()
            && self.files.iter().all(|f| plain(&f.name) && hex(&f.sha256, 64) && !f.path.contains(".."))
    }
}

/// The id of `repo`'s model: its short-list id, else `hf--<owner>--<repository>` in lower case.
pub fn id_for_repo(repo: &str) -> String {
    if let Some(m) = catalog().into_iter().find(|m| m.repo.eq_ignore_ascii_case(repo)) {
        return m.id;
    }
    let name: String = repo
        .to_lowercase()
        .replace('/', "--")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '-' })
        .collect();
    format!("{FOUND}{name}")
}

/// The manifest in `dir`, when it is there, reads and is sound.
pub fn read_manifest(dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    let m: Manifest = serde_json::from_str(&text).ok()?;
    m.sound().then_some(m)
}

pub fn write_manifest(dir: &Path, m: &Manifest) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(m).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(MANIFEST_FILE), json)
}

/// The model `id`: from the short list, or a found one's manifest in its folder under `models`.
pub fn resolve(models: &Path, id: &str) -> Option<Manifest> {
    catalog()
        .into_iter()
        .find(|m| m.id == id)
        .or_else(|| id.starts_with(FOUND).then(|| read_manifest(&models.join(id))).flatten().filter(|m| m.id == id))
}

/// The found models with a folder under `models` (downloaded or on their way), by name.
pub fn found_models(models: &Path) -> Vec<Manifest> {
    let mut out: Vec<Manifest> = std::fs::read_dir(models)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            id.starts_with(FOUND).then(|| read_manifest(&e.path())).flatten().filter(|m| m.id == id)
        })
        .collect();
    out.sort_by_key(|m| m.name.to_lowercase());
    out
}

/// A file of a repository, as Hugging Face lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoFile {
    pub path: String,
    pub size: u64,
}

/// Why CUE cannot run a repository's model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotRunnable {
    /// Not a TDT model in the onnx-asr layout: Whisper, CTC, RNN-T…
    Architecture,
    /// TDT, but in fp32 or fp16 only.
    NoInt8,
    /// TDT in separate encoder, decoder and joiner files.
    Layout,
}

/// Where a runnable repository keeps the three files CUE runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub encoder: RepoFile,
    pub decoder: RepoFile,
    pub vocab: RepoFile,
    /// `config.json` next to the encoder or at the root, when there is one.
    pub config: Option<String>,
}

impl Layout {
    pub fn size(&self) -> u64 {
        self.encoder.size + self.decoder.size + self.vocab.size
    }
}

fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// Whether CUE can run `repo`'s model, from its files and tags: one folder with the int8 encoder and decoder/joint, a
/// `vocab.txt` there or at the root, and a TDT model (by its tags or its name).
pub fn runnable(repo: &str, files: &[RepoFile], tags: &[String]) -> Result<Layout, NotRunnable> {
    let tdt = tags.iter().any(|t| t == "nemo-conformer-tdt") || repo.to_lowercase().contains("tdt");
    let find = |folder: &str, name: &str| files.iter().find(|f| split(&f.path) == (folder, name)).cloned();
    for enc in files.iter().filter(|f| split(&f.path).1 == ENCODER) {
        let folder = split(&enc.path).0;
        let (Some(decoder), Some(vocab)) = (find(folder, DECODER), find(folder, VOCAB).or_else(|| find("", VOCAB))) else {
            continue;
        };
        if !tdt {
            return Err(NotRunnable::Architecture);
        }
        let config = find(folder, "config.json").or_else(|| find("", "config.json")).map(|f| f.path);
        return Ok(Layout { encoder: enc.clone(), decoder, vocab, config });
    }
    let named = |part: &str| files.iter().any(|f| split(&f.path).1.contains(part) && f.path.ends_with(".onnx"));
    Err(if !tdt {
        NotRunnable::Architecture
    } else if named("decoder_joint-model") {
        NotRunnable::NoInt8
    } else if named("joiner") {
        NotRunnable::Layout
    } else {
        NotRunnable::Architecture
    })
}

/// An onnx-asr `config.json` CUE's features and decoder fit: a TDT model over 128 mel bands.
pub fn config_fits(json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(json).is_ok_and(|v| {
        v["model_type"] == "nemo-conformer-tdt" && v.get("features_size").is_none_or(|n| n == 128)
    })
}

/// The model in use, as the Drop, the main window and the welcome window show it (`cue://model`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ModelState {
    /// Not on this computer, and no download running.
    Missing,
    Downloading { done: u64, total: u64 },
    /// The files are here and checked; the model is loading.
    Loading,
    /// The model is loaded; `total` — its bytes on this computer.
    Ready { total: u64 },
    /// The download stopped (`done` of `total` bytes are kept for the next try) or the files did not load.
    Failed { done: u64, total: u64 },
    /// This processor cannot run the model.
    Unsupported,
}

/// A model on the Model page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum EntryState {
    /// The model recognizing now.
    InUse,
    Downloaded,
    Missing,
    /// A found model being pinned: its files asked of Hugging Face.
    Preparing,
    Downloading { done: u64, total: u64 },
    Loading,
    Failed { done: u64, total: u64 },
    /// Downloaded and checked, and still did not load.
    NotRunnable,
    /// The drive has too little room for the download.
    NoSpace { needed: u64, free: u64 },
}

/// A model on the Model page (`cue://models`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub languages: Vec<String>,
    pub size: u64,
    #[serde(flatten)]
    pub state: EntryState,
}

/// The Model page's list and the model in use (`cue://models`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelsView {
    pub current: String,
    pub entries: Vec<ModelEntry>,
    /// The short list's recommended model, where a list marks one (the local cleanup's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommended: Option<String>,
}

impl ModelEntry {
    pub fn new(m: &Manifest, state: EntryState) -> Self {
        Self { id: m.id.clone(), name: m.name.clone(), repo: m.repo.clone(), languages: m.languages.clone(), size: m.total(), state }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[&str]) -> Vec<RepoFile> {
        list.iter().map(|p| RepoFile { path: (*p).to_owned(), size: 10 }).collect()
    }

    fn tags(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| (*t).to_owned()).collect()
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cue-model-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn urls_name_the_commit_not_main() {
        let u = ultra();
        assert_eq!(
            u.url(&u.files[2]),
            "https://huggingface.co/Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx/resolve/b27f93c2238aa35e138996c005a62f1e8430f801/int8/encoder-model.int8.onnx"
        );
        assert_eq!(u.total(), 93_939 + 18_203_490 + 649_431_593);
        let v3 = &catalog()[1];
        assert_eq!(v3.url(&v3.files[2]), format!("https://huggingface.co/{}/resolve/{}/encoder-model.int8.onnx", v3.repo, v3.commit));
    }

    #[test]
    fn the_short_list_is_ultra_first_and_every_entry_is_sound() {
        let list = catalog();
        assert_eq!(list[0], ultra());
        assert_eq!(list[0].id, DEFAULT_ID);
        assert_eq!(list.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["Parakeet Ultra", "Parakeet v3", "Parakeet v2"]);
        assert!(list.iter().all(Manifest::sound));
        assert_eq!(list[2].languages, ["en"], "v2 is English only");
        assert!(list[1].languages.iter().any(|l| l == "ru"));
    }

    #[test]
    fn a_repository_gets_its_short_list_id_or_a_folder_name() {
        assert_eq!(id_for_repo("Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx"), DEFAULT_ID);
        assert_eq!(id_for_repo("istupakov/Parakeet-TDT-0.6b-v3-onnx"), "parakeet-v3-int8");
        assert_eq!(id_for_repo("myned-ai/Parakeet-TDT-0.6B-v3-Greek"), "hf--myned-ai--parakeet-tdt-0.6b-v3-greek");
    }

    #[test]
    fn onnx_asr_layouts_run() {
        // istupakov's: everything at the root.
        let root = files(&["config.json", "decoder_joint-model.int8.onnx", "decoder_joint-model.onnx", "encoder-model.int8.onnx", "nemo128.onnx", "vocab.txt"]);
        let l = runnable("istupakov/parakeet-tdt-0.6b-v2-onnx", &root, &tags(&["onnx", "nemo-conformer-tdt"])).unwrap();
        assert_eq!((l.encoder.path.as_str(), l.vocab.path.as_str(), l.config.as_deref()), ("encoder-model.int8.onnx", "vocab.txt", Some("config.json")));
        assert_eq!(l.size(), 30);
        // Olicorne's: int8/ with the vocabulary at the root.
        let ultra = files(&["config.json", "int8/decoder_joint-model.int8.onnx", "int8/encoder-model.int8.onnx", "int8/encoder-model.int8.onnx.zst", "fp16/encoder-model.fp16.onnx", "vocab.txt"]);
        let l = runnable("Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx", &ultra, &[]).unwrap();
        assert_eq!((l.encoder.path.as_str(), l.decoder.path.as_str(), l.vocab.path.as_str()), ("int8/encoder-model.int8.onnx", "int8/decoder_joint-model.int8.onnx", "vocab.txt"));
    }

    #[test]
    fn other_models_do_not_run_and_say_why() {
        let whisper = files(&["onnx/encoder_model_int8.onnx", "onnx/decoder_model_merged_int8.onnx", "tokenizer.json"]);
        assert_eq!(runnable("onnx-community/whisper-large-v3-turbo", &whisper, &[]), Err(NotRunnable::Architecture));
        let fp32 = files(&["decoder_joint-model.onnx", "encoder-model.onnx", "encoder-model.onnx.data", "vocab.txt"]);
        assert_eq!(runnable("someone/parakeet-tdt-0.6b-v3-fp32-onnx", &fp32, &[]), Err(NotRunnable::NoInt8));
        let sherpa = files(&["decoder.int8.onnx", "encoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"]);
        assert_eq!(runnable("csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", &sherpa, &[]), Err(NotRunnable::Layout));
        // The onnx-asr layout of an RNN-T or CTC model: the same file names, not TDT.
        let rnnt = files(&["decoder_joint-model.int8.onnx", "encoder-model.int8.onnx", "vocab.txt"]);
        assert_eq!(runnable("istupakov/stt_ru_fastconformer_hybrid_large_pc_onnx", &rnnt, &tags(&["nemo-conformer-rnnt"])), Err(NotRunnable::Architecture));
        // Files in two folders do not make one model.
        let split = files(&["a/encoder-model.int8.onnx", "b/decoder_joint-model.int8.onnx", "vocab.txt"]);
        assert!(runnable("x/parakeet-tdt", &split, &[]).is_err());
    }

    #[test]
    fn configs_of_tdt_over_128_bands_fit() {
        assert!(config_fits(r#"{"model_type": "nemo-conformer-tdt", "features_size": 128, "subsampling_factor": 8}"#));
        assert!(config_fits(r#"{"model_type": "nemo-conformer-tdt"}"#));
        assert!(!config_fits(r#"{"model_type": "nemo-conformer-rnnt", "features_size": 128}"#));
        assert!(!config_fits(r#"{"model_type": "nemo-conformer-tdt", "features_size": 80}"#));
        assert!(!config_fits("not json"));
    }

    #[test]
    fn a_found_model_keeps_its_manifest_in_its_folder() {
        let models = temp("found");
        let mut m = catalog()[1].clone();
        m.id = id_for_repo("myned-ai/parakeet-tdt-0.6b-v3-greek");
        m.name = "myned-ai/parakeet-tdt-0.6b-v3-greek".into();
        write_manifest(&models.join(&m.id), &m).unwrap();
        assert_eq!(resolve(&models, &m.id), Some(m.clone()));
        assert_eq!(found_models(&models), vec![m.clone()]);
        assert_eq!(resolve(&models, DEFAULT_ID), Some(ultra()), "the short list needs no folder");
        assert_eq!(resolve(&models, "hf--nobody--nothing"), None);
        // A folder of someone else's, a manifest naming another folder, one with a path for a name: not CUE's.
        std::fs::create_dir_all(models.join("hf--strange")).unwrap();
        let mut moved = m.clone();
        moved.id = "hf--other".into();
        write_manifest(&models.join("hf--elsewhere"), &moved).unwrap();
        let mut bad = m.clone();
        bad.id = "hf--bad".into();
        bad.files[0].name = "..\\..\\evil.dll".into();
        write_manifest(&models.join("hf--bad"), &bad).unwrap();
        let mut branch = m.clone();
        branch.id = "hf--branch".into();
        branch.commit = "main".into();
        write_manifest(&models.join("hf--branch"), &branch).unwrap();
        assert_eq!(found_models(&models), vec![m]);
        assert_eq!(resolve(&models, "hf--bad"), None);
        let _ = std::fs::remove_dir_all(&models);
    }

    /// The local cleanup's list names its recommended model; the speech list says nothing of one.
    #[test]
    fn models_view_names_the_recommended_only_when_there_is_one() {
        let v = ModelsView { current: "a".into(), entries: vec![], recommended: None };
        assert!(serde_json::to_value(&v).unwrap().get("recommended").is_none());
        let v = ModelsView { recommended: Some("llm--a--b--q4_k_m".into()), ..v };
        assert_eq!(serde_json::to_value(&v).unwrap()["recommended"], "llm--a--b--q4_k_m");
    }

    /// The cleanup's language models share the `models` folder; the speech list never shows them.
    #[test]
    fn found_models_skips_language_models() {
        let models = temp("found-llm");
        let mut m = catalog()[1].clone();
        m.id = "llm--qwen--qwen3-4b-gguf--q4_k_m".into();
        write_manifest(&models.join(&m.id), &m).unwrap();
        assert!(found_models(&models).is_empty());
        assert_eq!(resolve(&models, &m.id), None);
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn states_go_to_the_windows_by_name() {
        let json = |s: ModelState| serde_json::to_string(&s).unwrap();
        assert_eq!(json(ModelState::Downloading { done: 1, total: 2 }), r#"{"state":"downloading","done":1,"total":2}"#);
        assert_eq!(json(ModelState::Ready { total: 7 }), r#"{"state":"ready","total":7}"#);
        assert_eq!(json(ModelState::Failed { done: 0, total: 9 }), r#"{"state":"failed","done":0,"total":9}"#);
        assert_eq!(json(ModelState::Unsupported), r#"{"state":"unsupported"}"#);
    }

    #[test]
    fn entries_go_to_the_page_by_name() {
        let e = ModelEntry::new(&catalog()[2], EntryState::NoSpace { needed: 5, free: 1 });
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            format!(
                r#"{{"id":"parakeet-v2-int8","name":"Parakeet v2","repo":"istupakov/parakeet-tdt-0.6b-v2-onnx","languages":["en"],"size":{},"state":"noSpace","needed":5,"free":1}}"#,
                catalog()[2].total()
            )
        );
        let json = |s: EntryState| serde_json::to_value(ModelEntry::new(&ultra(), s)).unwrap()["state"].clone();
        assert_eq!(json(EntryState::InUse), "inUse");
        assert_eq!(json(EntryState::NotRunnable), "notRunnable");
        assert_eq!(serde_json::to_string(&NotRunnable::NoInt8).unwrap(), r#""noInt8""#);
    }
}

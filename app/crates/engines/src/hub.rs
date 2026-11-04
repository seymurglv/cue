//! Hugging Face for the Model page (spec "Choosing the speech model"): the search, which tells for each repository
//! whether CUE can run its model (`cue_core::model::runnable`), and pinning a found model when it is chosen: the
//! repository at its current commit, the large files' SHA-256 from its LFS data, `vocab.txt` downloaded at once and
//! checked against its git blob hash, and the manifest written into the model's folder. The download that follows is
//! the short list's own (`download`). The cleanup's language models (docs/specs/2026-11-04-local-cleanup.md) are
//! searched and pinned the same way: GGUF repositories, one file each (`cue_core::llm::runnable`).
use std::path::Path;
use std::time::Duration;

use cue_core::llm::{self, GgufMeta, LlmNotRunnable};
use cue_core::model::{self, DECODER, ENCODER, Manifest, ModelFile, NotRunnable, RepoFile, VOCAB};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};

use crate::EngineError;
use crate::error::{checked, net};

pub const HUB: &str = "https://huggingface.co";
/// Repositories the search shows.
pub const SHOWN: usize = 10;
/// Repositories asked for, before the gated ones are left out.
const ASKED: usize = 30;
const TIMEOUT: Duration = Duration::from_secs(20);

/// A repository the search found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub repo: String,
    pub downloads: u64,
    /// Bytes of the three files CUE would download, for a runnable one whose files were listed.
    pub size: Option<u64>,
    /// Why CUE cannot run it; `None` — it can.
    pub reason: Option<NotRunnable>,
    pub languages: Vec<String>,
}

/// A repository as Hugging Face lists it.
#[derive(Deserialize)]
struct Listed {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    siblings: Vec<Sibling>,
    /// `false`, or how access is granted: such a repository cannot be downloaded without signing in.
    #[serde(default)]
    gated: serde_json::Value,
    sha: Option<String>,
    /// What the repository's model does, when it says: anything but speech recognition is not for this page.
    pipeline_tag: Option<String>,
    #[serde(rename = "cardData", default)]
    card: Option<Card>,
    /// What Hugging Face read from a GGUF repository's file: its architecture and chat template.
    #[serde(default)]
    gguf: Option<GgufListed>,
}

#[derive(Deserialize)]
struct GgufListed {
    architecture: Option<String>,
    chat_template: Option<String>,
}

#[derive(Deserialize)]
struct Sibling {
    rfilename: String,
    #[serde(default)]
    size: u64,
    #[serde(rename = "blobId")]
    blob: Option<String>,
    lfs: Option<Lfs>,
}

#[derive(Deserialize)]
struct Lfs {
    sha256: String,
}

#[derive(Deserialize)]
struct Card {
    license: Option<serde_json::Value>,
}

impl Listed {
    fn open(&self) -> bool {
        matches!(self.gated, serde_json::Value::Null | serde_json::Value::Bool(false))
    }

    fn speech(&self) -> bool {
        self.pipeline_tag.as_deref().is_none_or(|p| p == "automatic-speech-recognition")
    }

    /// A text model (a chat model), or one that names no pipeline.
    fn text(&self) -> bool {
        self.pipeline_tag.as_deref().is_none_or(|p| p == "text-generation")
    }

    fn gguf_meta(&self) -> GgufMeta {
        let g = self.gguf.as_ref();
        GgufMeta {
            architecture: g.and_then(|g| g.architecture.clone()),
            chat_template: g.and_then(|g| g.chat_template.as_deref()).is_some_and(|t| !t.trim().is_empty()),
        }
    }

    fn llm_found(&self) -> LlmFound {
        let pick = llm::runnable(&self.files(), &self.gguf_meta());
        LlmFound {
            repo: self.id.clone(),
            downloads: self.downloads,
            size: None,
            quant: pick.as_ref().ok().map(|p| p.quant.to_owned()),
            reason: pick.err(),
        }
    }

    fn license(&self) -> String {
        self.card.as_ref().and_then(|c| c.license.as_ref()).and_then(|v| v.as_str()).unwrap_or_default().to_owned()
    }

    fn files(&self) -> Vec<RepoFile> {
        self.siblings.iter().map(|s| RepoFile { path: s.rfilename.clone(), size: s.size }).collect()
    }

    /// The language tags: two-letter codes.
    fn languages(&self) -> Vec<String> {
        self.tags.iter().filter(|t| t.len() == 2 && t.chars().all(|c| c.is_ascii_lowercase())).cloned().collect()
    }

    fn sibling(&self, path: &str) -> Option<&Sibling> {
        self.siblings.iter().find(|s| s.rfilename == path)
    }

    fn found(&self) -> Found {
        let runnable = model::runnable(&self.id, &self.files(), &self.tags);
        Found {
            repo: self.id.clone(),
            downloads: self.downloads,
            size: None,
            reason: runnable.err(),
            languages: self.languages(),
        }
    }
}

/// The search's answer: the speech models that can be downloaded without signing in, the first `SHOWN`.
pub fn parse_search(json: &str) -> Result<Vec<Found>, EngineError> {
    let listed: Vec<Listed> = serde_json::from_str(json).map_err(|e| EngineError::Protocol(format!("search: {e}")))?;
    Ok(listed.iter().filter(|l| l.open() && l.speech()).take(SHOWN).map(Listed::found).collect())
}

/// `s` as a URL's query value.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

async fn get(http: &reqwest::Client, url: &str) -> Result<reqwest::Response, EngineError> {
    let resp = http.get(url).timeout(TIMEOUT).send().await.map_err(net)?;
    checked(resp, "").await
}

async fn get_text(http: &reqwest::Client, url: &str) -> Result<String, EngineError> {
    get(http, url).await?.text().await.map_err(net)
}

/// ONNX repositories matching `query`, by downloads; a runnable one with its size.
pub async fn search(http: &reqwest::Client, query: &str) -> Result<Vec<Found>, EngineError> {
    search_at(http, HUB, query).await
}

async fn search_at(http: &reqwest::Client, base: &str, query: &str) -> Result<Vec<Found>, EngineError> {
    let url = format!(
        "{base}/api/models?search={}&filter=onnx&sort=downloads&limit={ASKED}&expand%5B%5D=siblings&expand%5B%5D=downloads&expand%5B%5D=tags&expand%5B%5D=gated&expand%5B%5D=pipeline_tag",
        encode(query.trim())
    );
    let mut found = parse_search(&get_text(http, &url).await?)?;
    // The search lists files without sizes: a runnable one's are asked for, all at once.
    let sizes = futures_util::future::join_all(found.iter().map(|f| async {
        if f.reason.is_some() {
            return None;
        }
        let l = details(http, base, &f.repo).await.ok()?;
        model::runnable(&l.id, &l.files(), &l.tags).ok().map(|layout| layout.size())
    }))
    .await;
    for (f, size) in found.iter_mut().zip(sizes) {
        f.size = size;
    }
    Ok(found)
}

/// The repository at its current commit, its files with their sizes and hashes.
async fn details(http: &reqwest::Client, base: &str, repo: &str) -> Result<Listed, EngineError> {
    let text = get_text(http, &format!("{base}/api/models/{repo}?blobs=true")).await?;
    serde_json::from_str(&text).map_err(|e| EngineError::Protocol(format!("{repo}: {e}")))
}

/// Why a found model could not be pinned.
#[derive(Debug)]
pub enum PinError {
    NotRunnable(NotRunnable),
    Failed(EngineError),
}

impl From<EngineError> for PinError {
    fn from(e: EngineError) -> Self {
        Self::Failed(e)
    }
}

/// The git hash of a file's content, as Hugging Face lists it for a file outside LFS.
pub fn blob_hash(content: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", content.len()));
    h.update(content);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(content: &[u8]) -> String {
    Sha256::digest(content).iter().map(|b| format!("{b:02x}")).collect()
}

/// `repo` pinned at its current commit: its manifest, written with its `vocab.txt` into its folder under `models`.
pub async fn pin(http: &reqwest::Client, repo: &str, models: &Path) -> Result<Manifest, PinError> {
    pin_at(http, HUB, repo, models).await
}

async fn pin_at(http: &reqwest::Client, base: &str, repo: &str, models: &Path) -> Result<Manifest, PinError> {
    let l = details(http, base, repo).await?;
    let protocol = |what: &str| PinError::Failed(EngineError::Protocol(format!("{}: {what}", l.id)));
    if !l.open() {
        return Err(protocol("needs signing in"));
    }
    let commit = l.sha.clone().filter(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())).ok_or_else(|| protocol("no commit"))?;
    let layout = model::runnable(&l.id, &l.files(), &l.tags).map_err(PinError::NotRunnable)?;
    let resolve = |path: &str| format!("{base}/{}/resolve/{commit}/{path}", l.id);
    if let Some(config) = &layout.config
        && !model::config_fits(&get_text(http, &resolve(config)).await?)
    {
        return Err(PinError::NotRunnable(NotRunnable::Architecture));
    }
    let lfs = |f: &RepoFile, name: &str| -> Result<ModelFile, PinError> {
        let sha = l.sibling(&f.path).and_then(|s| s.lfs.as_ref()).map(|lfs| lfs.sha256.to_lowercase()).ok_or_else(|| protocol("a model file without its hash"))?;
        Ok(ModelFile { path: f.path.clone(), name: name.to_owned(), size: f.size, sha256: sha })
    };
    let (encoder, decoder) = (lfs(&layout.encoder, ENCODER)?, lfs(&layout.decoder, DECODER)?);
    // The vocabulary is small and usually outside LFS: checked against its blob hash, or its LFS hash when it is in LFS.
    let vocab = get(http, &resolve(&layout.vocab.path)).await?.bytes().await.map_err(net)?;
    let listed = l.sibling(&layout.vocab.path).ok_or_else(|| protocol("no vocab.txt"))?;
    let hash = sha256(&vocab);
    let fits = match (&listed.lfs, &listed.blob) {
        (Some(lfs), _) => lfs.sha256.eq_ignore_ascii_case(&hash),
        (None, Some(blob)) => blob.eq_ignore_ascii_case(&blob_hash(&vocab)),
        (None, None) => false,
    };
    if !fits || vocab.len() as u64 != layout.vocab.size {
        return Err(protocol("vocab.txt is not the one listed"));
    }
    let id = model::id_for_repo(&l.id);
    let license = l.license();
    let m = Manifest {
        id: id.clone(),
        name: l.id.clone(),
        repo: l.id.clone(),
        commit,
        files: vec![ModelFile { path: layout.vocab.path.clone(), name: VOCAB.to_owned(), size: layout.vocab.size, sha256: hash }, decoder, encoder],
        languages: l.languages(),
        license,
    };
    let dir = models.join(&id);
    let write = |e: std::io::Error| PinError::Failed(EngineError::Model(format!("model folder: {e}")));
    model::write_manifest(&dir, &m).map_err(write)?;
    std::fs::write(dir.join(VOCAB), &vocab).map_err(write)?;
    Ok(m)
}

/// A GGUF repository the search found, for the cleanup on this computer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmFound {
    pub repo: String,
    pub downloads: u64,
    /// Bytes of the file CUE would download, once its details were read.
    pub size: Option<u64>,
    /// The quantization CUE would take (`Q4_K_M`…); `None` when it cannot run.
    pub quant: Option<String>,
    /// Why CUE cannot run it; `None` — it can.
    pub reason: Option<LlmNotRunnable>,
}

/// The search's answer: the text models that can be downloaded without signing in, the first `SHOWN`.
pub fn parse_llm_search(json: &str) -> Result<Vec<LlmFound>, EngineError> {
    let listed: Vec<Listed> = serde_json::from_str(json).map_err(|e| EngineError::Protocol(format!("search: {e}")))?;
    Ok(listed.iter().filter(|l| l.open() && l.text()).take(SHOWN).map(Listed::llm_found).collect())
}

/// GGUF repositories matching `query`, by downloads; a runnable one with the size of the file CUE would take.
pub async fn search_llms(http: &reqwest::Client, query: &str) -> Result<Vec<LlmFound>, EngineError> {
    search_llms_at(http, HUB, query).await
}

async fn search_llms_at(http: &reqwest::Client, base: &str, query: &str) -> Result<Vec<LlmFound>, EngineError> {
    let url = format!(
        "{base}/api/models?search={}&filter=gguf&sort=downloads&limit={ASKED}&expand%5B%5D=siblings&expand%5B%5D=downloads&expand%5B%5D=tags&expand%5B%5D=gated&expand%5B%5D=pipeline_tag&expand%5B%5D=gguf",
        encode(query.trim())
    );
    let mut found = parse_llm_search(&get_text(http, &url).await?)?;
    // The search lists files without sizes: a runnable one's are asked for, all at once, and checked again with them.
    let checked = futures_util::future::join_all(found.iter().map(|f| async {
        if f.reason.is_some() {
            return None;
        }
        let l = details(http, base, &f.repo).await.ok()?;
        Some(llm::runnable(&l.files(), &l.gguf_meta()))
    }))
    .await;
    for (f, pick) in found.iter_mut().zip(checked) {
        match pick {
            Some(Ok(p)) => (f.size, f.quant) = (Some(p.file.size), Some(p.quant.to_owned())),
            Some(Err(reason)) => (f.reason, f.quant) = (Some(reason), None),
            None => {}
        }
    }
    Ok(found)
}

/// Why a language model could not be pinned.
#[derive(Debug)]
pub enum LlmPinError {
    NotRunnable(LlmNotRunnable),
    Failed(EngineError),
}

impl From<EngineError> for LlmPinError {
    fn from(e: EngineError) -> Self {
        Self::Failed(e)
    }
}

/// `repo`'s GGUF pinned at its current commit: its manifest (the one file, its size and LFS SHA-256), written into its
/// folder under `models`. Nothing is downloaded here.
pub async fn pin_llm(http: &reqwest::Client, repo: &str, models: &Path) -> Result<Manifest, LlmPinError> {
    pin_llm_at(http, HUB, repo, models).await
}

async fn pin_llm_at(http: &reqwest::Client, base: &str, repo: &str, models: &Path) -> Result<Manifest, LlmPinError> {
    let l = details(http, base, repo).await?;
    let protocol = |what: &str| LlmPinError::Failed(EngineError::Protocol(format!("{}: {what}", l.id)));
    if !l.open() {
        return Err(protocol("needs signing in"));
    }
    let commit = l.sha.clone().filter(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())).ok_or_else(|| protocol("no commit"))?;
    let pick = llm::runnable(&l.files(), &l.gguf_meta()).map_err(LlmPinError::NotRunnable)?;
    let sha = l
        .sibling(&pick.file.path)
        .and_then(|s| s.lfs.as_ref())
        .map(|lfs| lfs.sha256.to_lowercase())
        .ok_or_else(|| protocol("a model file without its hash"))?;
    let name = pick.file.path.rsplit('/').next().unwrap_or(&pick.file.path).to_owned();
    let id = llm::id_for(&l.id, pick.quant);
    let m = Manifest {
        id: id.clone(),
        name: l.id.clone(),
        repo: l.id.clone(),
        commit,
        files: vec![ModelFile { path: pick.file.path.clone(), name, size: pick.file.size, sha256: sha }],
        languages: l.languages(),
        license: l.license(),
    };
    model::write_manifest(&models.join(&id), &m).map_err(|e| LlmPinError::Failed(EngineError::Model(format!("model folder: {e}"))))?;
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Reply, fake_api, reply};

    const VOCAB_TEXT: &str = "▁the 0\n▁a 1\n<blk> 2\n";

    /// A search answer: a runnable one, Whisper, sherpa-onnx's layout, and a gated one.
    fn listing() -> String {
        serde_json::json!([
            {"id": "someone/parakeet-tdt-0.6b-v3-greek", "downloads": 134, "tags": ["onnx", "nemo-conformer-tdt", "el"],
             "siblings": [{"rfilename": "config.json"}, {"rfilename": "decoder_joint-model.int8.onnx"},
                          {"rfilename": "encoder-model.int8.onnx"}, {"rfilename": "vocab.txt"}]},
            {"id": "onnx-community/whisper-small", "downloads": 9000, "tags": ["onnx", "en", "ru"],
             "siblings": [{"rfilename": "onnx/encoder_model_int8.onnx"}, {"rfilename": "onnx/decoder_model_merged_int8.onnx"}]},
            {"id": "csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", "downloads": 5,
             "siblings": [{"rfilename": "encoder.int8.onnx"}, {"rfilename": "decoder.int8.onnx"}, {"rfilename": "joiner.int8.onnx"}]},
            {"id": "someone/handwritten-greek-ocr", "downloads": 25, "pipeline_tag": "image-to-text",
             "siblings": [{"rfilename": "model.onnx"}]},
            {"id": "corp/parakeet-tdt-private", "downloads": 1, "gated": "manual",
             "siblings": [{"rfilename": "decoder_joint-model.int8.onnx"}, {"rfilename": "encoder-model.int8.onnx"}, {"rfilename": "vocab.txt"}]}
        ])
        .to_string()
    }

    /// The runnable one's details at a commit: sizes, the LFS hashes, the vocabulary's blob hash.
    fn details_json(vocab_blob: &str) -> String {
        serde_json::json!({
            "id": "someone/parakeet-tdt-0.6b-v3-greek", "sha": "c314e68a3935a5bb8e398681790ff13efb99432f",
            "tags": ["onnx", "nemo-conformer-tdt", "el"], "cardData": {"license": "cc-by-4.0"},
            "siblings": [
                {"rfilename": "config.json", "size": 97, "blobId": "02a7"},
                {"rfilename": "decoder_joint-model.int8.onnx", "size": 1000, "lfs": {"sha256": "A".repeat(64), "size": 1000}},
                {"rfilename": "encoder-model.int8.onnx", "size": 20000, "lfs": {"sha256": "b".repeat(64), "size": 20000}},
                {"rfilename": "vocab.txt", "size": VOCAB_TEXT.len(), "blobId": vocab_blob}
            ]
        })
        .to_string()
    }

    const TDT_CONFIG: &str = r#"{"model_type": "nemo-conformer-tdt", "features_size": 128}"#;

    fn folder(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("cue-hub-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn the_search_tells_which_can_run() {
        let found = parse_search(&listing()).unwrap();
        let shown: Vec<_> = found.iter().map(|f| (f.repo.as_str(), f.reason)).collect();
        assert_eq!(
            shown,
            [
                ("someone/parakeet-tdt-0.6b-v3-greek", None),
                ("onnx-community/whisper-small", Some(NotRunnable::Architecture)),
                ("csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", Some(NotRunnable::Layout)),
            ],
            "a gated repository and one that is not about speech are left out"
        );
        assert_eq!(found[0].languages, ["el"]);
        assert_eq!(found[1].languages, ["en", "ru"]);
        assert!(parse_search("{}").is_err());
    }

    #[test]
    fn a_blob_hash_is_gits() {
        // `git hash-object` of a file holding "hello\n".
        assert_eq!(blob_hash(b"hello\n"), "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[tokio::test]
    async fn runnable_results_get_their_size() {
        let api = fake_api(|_, path, _| {
            if path.starts_with("/api/models?search=greek%20tdt&filter=onnx") {
                reply(200, &listing())
            } else if path == "/api/models/someone/parakeet-tdt-0.6b-v3-greek?blobs=true" {
                reply(200, &details_json("x"))
            } else {
                reply(404, "{}")
            }
        })
        .await;
        let found = search_at(&reqwest::Client::new(), &api.base, " greek tdt ").await.unwrap();
        assert_eq!(found[0].size, Some(1000 + 20000 + VOCAB_TEXT.len() as u64));
        assert_eq!(found[1].size, None);
        assert_eq!(api.calls().len(), 2, "only the runnable one's files are asked for");
    }

    #[tokio::test]
    async fn no_connection_is_a_network_error() {
        let err = search_at(&reqwest::Client::new(), "http://127.0.0.1:9", "x").await.unwrap_err();
        assert!(matches!(err, EngineError::Network(_)), "{err:?}");
    }

    fn pin_api(vocab_blob: String, config: &'static str) -> impl Fn(&str, &str, usize) -> Reply + Send + Sync + 'static {
        move |_, path, _| match path {
            "/api/models/someone/parakeet-tdt-0.6b-v3-greek?blobs=true" => reply(200, &details_json(&vocab_blob)),
            "/someone/parakeet-tdt-0.6b-v3-greek/resolve/c314e68a3935a5bb8e398681790ff13efb99432f/config.json" => reply(200, config),
            "/someone/parakeet-tdt-0.6b-v3-greek/resolve/c314e68a3935a5bb8e398681790ff13efb99432f/vocab.txt" => reply(200, VOCAB_TEXT),
            _ => reply(404, "{}"),
        }
    }

    #[tokio::test]
    async fn a_chosen_model_is_pinned_at_its_commit() {
        let api = fake_api(pin_api(blob_hash(VOCAB_TEXT.as_bytes()), TDT_CONFIG)).await;
        let models = folder("pin");
        let m = pin_at(&reqwest::Client::new(), &api.base, "someone/parakeet-tdt-0.6b-v3-greek", &models).await.unwrap();
        assert_eq!(m.id, "hf--someone--parakeet-tdt-0.6b-v3-greek");
        assert_eq!(m.commit, "c314e68a3935a5bb8e398681790ff13efb99432f");
        assert_eq!((m.languages.clone(), m.license.as_str()), (vec!["el".to_owned()], "cc-by-4.0"));
        let names: Vec<_> = m.files.iter().map(|f| (f.name.as_str(), f.size)).collect();
        assert_eq!(names, [(VOCAB, VOCAB_TEXT.len() as u64), (DECODER, 1000), (ENCODER, 20000)]);
        assert_eq!(m.files[1].sha256, "a".repeat(64), "hashes in lower case");
        assert_eq!(m.files[0].sha256, sha256(VOCAB_TEXT.as_bytes()));
        let dir = models.join(&m.id);
        assert_eq!(model::read_manifest(&dir), Some(m.clone()));
        assert_eq!(std::fs::read_to_string(dir.join(VOCAB)).unwrap(), VOCAB_TEXT);
        assert_eq!(model::resolve(&models, &m.id), Some(m));
        std::fs::remove_dir_all(models).unwrap();
    }

    #[tokio::test]
    async fn a_config_of_another_model_type_is_not_runnable() {
        let api = fake_api(pin_api(blob_hash(VOCAB_TEXT.as_bytes()), r#"{"model_type": "nemo-conformer-rnnt"}"#)).await;
        let models = folder("rnnt");
        let r = pin_at(&reqwest::Client::new(), &api.base, "someone/parakeet-tdt-0.6b-v3-greek", &models).await;
        assert!(matches!(r, Err(PinError::NotRunnable(NotRunnable::Architecture))), "{r:?}");
        assert!(!models.exists(), "nothing is written");
    }

    #[tokio::test]
    async fn a_vocabulary_unlike_its_listing_is_refused() {
        let api = fake_api(pin_api("0".repeat(40), TDT_CONFIG)).await;
        let models = folder("vocab");
        let r = pin_at(&reqwest::Client::new(), &api.base, "someone/parakeet-tdt-0.6b-v3-greek", &models).await;
        assert!(matches!(r, Err(PinError::Failed(EngineError::Protocol(_)))), "{r:?}");
        assert!(!models.exists());
    }

    // ---- the cleanup's language models (GGUF) ----

    /// A GGUF search answer: two runnable, a split one, a gated one, one about speech, one about images.
    fn llm_listing() -> String {
        let gguf = |arch: &str| serde_json::json!({"architecture": arch, "chat_template": "{% for m in messages %}{% endfor %}"});
        serde_json::json!([
            {"id": "Qwen/Qwen3-4B-GGUF", "downloads": 900, "pipeline_tag": "text-generation", "gguf": gguf("qwen3"),
             "siblings": [{"rfilename": "Qwen3-4B-Q4_K_M.gguf"}, {"rfilename": "Qwen3-4B-Q8_0.gguf"}, {"rfilename": "README.md"}]},
            {"id": "someone/tiny-GGUF", "downloads": 40, "gguf": gguf("llama"), "siblings": [{"rfilename": "tiny-Q4_0.gguf"}]},
            {"id": "big/split-GGUF", "downloads": 30, "pipeline_tag": "text-generation", "gguf": gguf("qwen3"),
             "siblings": [{"rfilename": "m-Q4_K_M-00001-of-00002.gguf"}, {"rfilename": "m-Q4_K_M-00002-of-00002.gguf"}]},
            {"id": "corp/private-GGUF", "downloads": 20, "gated": "auto", "gguf": gguf("llama"), "siblings": [{"rfilename": "p-Q4_K_M.gguf"}]},
            {"id": "someone/whisper-GGUF", "downloads": 10, "pipeline_tag": "automatic-speech-recognition", "siblings": [{"rfilename": "w-Q4_K_M.gguf"}]},
            {"id": "someone/vision-GGUF", "downloads": 5, "pipeline_tag": "image-text-to-text", "gguf": gguf("gemma3"),
             "siblings": [{"rfilename": "v-Q4_K_M.gguf"}, {"rfilename": "mmproj-v-f16.gguf"}]}
        ])
        .to_string()
    }

    /// Qwen3 4B at a commit, with the sizes and LFS hashes of its files.
    fn llm_details() -> String {
        serde_json::json!({
            "id": "Qwen/Qwen3-4B-GGUF", "sha": "0123456789abcdef0123456789abcdef01234567",
            "pipeline_tag": "text-generation", "cardData": {"license": "apache-2.0"},
            "gguf": {"architecture": "qwen3", "chat_template": "{% for m in messages %}{% endfor %}"},
            "siblings": [
                {"rfilename": "Qwen3-4B-Q4_K_M.gguf", "size": 2_500_000_000u64, "lfs": {"sha256": "C".repeat(64), "size": 2_500_000_000u64}},
                {"rfilename": "Qwen3-4B-Q8_0.gguf", "size": 4_300_000_000u64, "lfs": {"sha256": "d".repeat(64), "size": 4_300_000_000u64}},
                {"rfilename": "README.md", "size": 900, "blobId": "aa"}
            ]
        })
        .to_string()
    }

    const LLM_SEARCH: &str = "/api/models?search=qwen&filter=gguf&sort=downloads&limit=30&expand%5B%5D=siblings&expand%5B%5D=downloads&expand%5B%5D=tags&expand%5B%5D=gated&expand%5B%5D=pipeline_tag&expand%5B%5D=gguf";

    #[test]
    fn llm_search_keeps_open_text_models() {
        let found = parse_llm_search(&llm_listing()).unwrap();
        let shown: Vec<_> = found.iter().map(|f| (f.repo.as_str(), f.reason, f.quant.as_deref())).collect();
        assert_eq!(
            shown,
            [
                ("Qwen/Qwen3-4B-GGUF", None, Some("Q4_K_M")),
                ("someone/tiny-GGUF", None, Some("Q4_0")),
                ("big/split-GGUF", Some(LlmNotRunnable::SeveralFiles), None),
            ],
            "gated, speech and image models are left out"
        );
        assert!(found.iter().all(|f| f.size.is_none()), "the search lists no sizes");
    }

    #[tokio::test]
    async fn llm_search_asks_for_gguf() {
        let api = fake_api(|_, path, _| {
            if path == LLM_SEARCH {
                reply(200, &llm_listing())
            } else if path == "/api/models/Qwen/Qwen3-4B-GGUF?blobs=true" {
                reply(200, &llm_details())
            } else {
                reply(404, "{}")
            }
        })
        .await;
        let found = search_llms_at(&reqwest::Client::new(), &api.base, " qwen ").await.unwrap();
        assert_eq!(found[0].size, Some(2_500_000_000), "the picked file's size");
        assert_eq!(found[1].size, None, "its details did not come");
        assert_eq!(found[2].size, None);
        assert!(api.calls().contains(&format!("GET {LLM_SEARCH}")));
    }

    #[tokio::test]
    async fn llm_search_says_why_not() {
        let mut too_big: serde_json::Value = serde_json::from_str(&llm_details()).unwrap();
        for s in too_big["siblings"].as_array_mut().unwrap() {
            s["size"] = serde_json::json!(7_000_000_000u64);
        }
        let too_big = too_big.to_string();
        let api = fake_api(move |_, path, _| {
            if path == LLM_SEARCH {
                reply(200, &llm_listing())
            } else if path == "/api/models/Qwen/Qwen3-4B-GGUF?blobs=true" {
                reply(200, &too_big)
            } else {
                reply(404, "{}")
            }
        })
        .await;
        let found = search_llms_at(&reqwest::Client::new(), &api.base, "qwen").await.unwrap();
        assert_eq!((found[0].reason, found[0].size, found[0].quant.as_deref()), (Some(LlmNotRunnable::TooLarge), None, None));
    }

    #[tokio::test]
    async fn pin_llm_writes_a_sound_manifest() {
        let api = fake_api(|_, path, _| {
            if path == "/api/models/Qwen/Qwen3-4B-GGUF?blobs=true" { reply(200, &llm_details()) } else { reply(404, "{}") }
        })
        .await;
        let models = folder("pin-llm");
        let m = pin_llm_at(&reqwest::Client::new(), &api.base, "Qwen/Qwen3-4B-GGUF", &models).await.unwrap();
        assert_eq!(m.id, "llm--qwen--qwen3-4b-gguf--q4_k_m");
        assert_eq!((m.repo.as_str(), m.commit.as_str(), m.license.as_str()), ("Qwen/Qwen3-4B-GGUF", "0123456789abcdef0123456789abcdef01234567", "apache-2.0"));
        assert_eq!(m.files.len(), 1);
        let f = &m.files[0];
        assert_eq!((f.path.as_str(), f.name.as_str(), f.size, f.sha256.as_str()), ("Qwen3-4B-Q4_K_M.gguf", "Qwen3-4B-Q4_K_M.gguf", 2_500_000_000, "c".repeat(64).as_str()));
        assert_eq!(cue_core::llm::resolve(&models, &m.id), Some(m.clone()), "the manifest is written and sound");
        let names: Vec<_> = std::fs::read_dir(models.join(&m.id)).unwrap().flatten().map(|e| e.file_name().into_string().unwrap()).collect();
        assert_eq!(names, ["manifest.json"], "nothing else is downloaded");
        assert_eq!(api.calls(), ["GET /api/models/Qwen/Qwen3-4B-GGUF?blobs=true"]);
        let _ = std::fs::remove_dir_all(&models);
    }

    #[tokio::test]
    async fn pin_llm_refuses_gated_and_unrunnable() {
        let gated = serde_json::json!({"id": "corp/private-GGUF", "sha": "0123456789abcdef0123456789abcdef01234567", "gated": "manual",
            "gguf": {"architecture": "llama", "chat_template": "x"}, "siblings": [{"rfilename": "p-Q4_K_M.gguf", "size": 1, "lfs": {"sha256": "a".repeat(64)}}]})
        .to_string();
        let split = serde_json::json!({"id": "big/split-GGUF", "sha": "0123456789abcdef0123456789abcdef01234567",
            "gguf": {"architecture": "qwen3", "chat_template": "x"},
            "siblings": [{"rfilename": "m-Q4_K_M-00001-of-00002.gguf", "size": 1}, {"rfilename": "m-Q4_K_M-00002-of-00002.gguf", "size": 1}]})
        .to_string();
        let api = fake_api(move |_, path, _| match path {
            "/api/models/corp/private-GGUF?blobs=true" => reply(200, &gated),
            "/api/models/big/split-GGUF?blobs=true" => reply(200, &split),
            _ => reply(404, "{}"),
        })
        .await;
        let models = folder("pin-llm-refused");
        let http = reqwest::Client::new();
        assert!(matches!(pin_llm_at(&http, &api.base, "corp/private-GGUF", &models).await, Err(LlmPinError::Failed(_))));
        assert!(matches!(pin_llm_at(&http, &api.base, "big/split-GGUF", &models).await, Err(LlmPinError::NotRunnable(LlmNotRunnable::SeveralFiles))));
        assert!(!models.exists(), "nothing is written");
    }
}

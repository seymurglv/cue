//! The language models the cleanup can run on this computer (docs/specs/2026-11-04-local-cleanup.md): which GGUF
//! file of a Hugging Face repository CUE takes, why one cannot run, the ids and folders of these models, and the short
//! list pinned here. A model's manifest is the speech models' `Manifest`, with its one file.
use std::path::Path;

use serde::Serialize;

pub use crate::model::{Manifest, RepoFile};
use crate::model::read_manifest;

/// The quantizations CUE takes, the first found in this order.
pub const QUANTS: [&str; 5] = ["Q4_K_M", "Q4_K_S", "Q5_K_M", "Q8_0", "Q4_0"];
/// The largest file CUE downloads for the cleanup: 6 GB.
pub const MAX_BYTES: u64 = 6_000_000_000;
/// The architectures the linked llama.cpp loads, as GGUF metadata names them (`general.architecture`).
pub const ARCHITECTURES: &[&str] = &[
    "llama", "mistral3", "qwen2", "qwen3", "qwen3moe", "gemma", "gemma2", "gemma3", "gemma3n", "gemma4", "phi3", "granite",
    "olmo2", "exaone", "smollm3",
];
/// The prefix of a language model's id and folder: `llm--<owner>--<repository>--<quant>`.
const PREFIX: &str = "llm--";

/// Why a repository's model cannot run in CUE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LlmNotRunnable {
    /// Only split into several files.
    SeveralFiles,
    NoChatTemplate,
    /// Larger than [`MAX_BYTES`] in every quantization CUE takes.
    TooLarge,
    Architecture,
    /// None of [`QUANTS`].
    NoQuant,
}

/// What Hugging Face's listing says of a repository's GGUF: its architecture, and whether it carries a chat template.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GgufMeta {
    pub architecture: Option<String>,
    pub chat_template: bool,
}

/// The file CUE downloads, and its quantization as [`QUANTS`] writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub file: RepoFile,
    pub quant: &'static str,
}

/// The file name without folders and the `.gguf` ending, when it is a GGUF CUE could take (not a vision projector).
fn gguf_stem(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    (lower.ends_with(".gguf") && !lower.starts_with("mmproj")).then(|| &name[..name.len() - ".gguf".len()])
}

/// A part of a split model: `<name>-00001-of-00002`.
fn split_part(stem: &str) -> Option<&str> {
    let mut parts = stem.rsplitn(4, '-');
    let (last, of, nth, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    let five = |s: &str| s.len() == 5 && s.chars().all(|c| c.is_ascii_digit());
    (five(last) && of == "of" && five(nth)).then_some(rest)
}

/// The quantization a stem ends with (`…-Q4_K_M`, `….q4_k_m`), if CUE takes it.
fn quant_of(stem: &str) -> Option<&'static str> {
    let last = stem.rsplit(['-', '.']).next()?;
    QUANTS.iter().copied().find(|q| q.eq_ignore_ascii_case(last))
}

/// The file CUE runs from a repository with these files, or why it cannot.
pub fn runnable(files: &[RepoFile], meta: &GgufMeta) -> Result<Pick, LlmNotRunnable> {
    if meta.architecture.as_deref().is_some_and(|a| !ARCHITECTURES.contains(&a)) {
        return Err(LlmNotRunnable::Architecture);
    }
    if !meta.chat_template {
        return Err(LlmNotRunnable::NoChatTemplate);
    }
    let mut whole: Vec<(usize, &RepoFile, &'static str)> = Vec::new();
    let mut split = false;
    for f in files {
        let Some(stem) = gguf_stem(&f.path) else { continue };
        match split_part(stem) {
            Some(rest) => split |= quant_of(rest).is_some(),
            None => {
                if let Some(q) = quant_of(stem) {
                    whole.push((QUANTS.iter().position(|x| *x == q).unwrap_or(QUANTS.len()), f, q));
                }
            }
        }
    }
    whole.sort_by_key(|(rank, f, _)| (*rank, f.path.clone()));
    if let Some((_, f, q)) = whole.iter().find(|(_, f, _)| f.size <= MAX_BYTES) {
        return Ok(Pick { file: (*f).clone(), quant: q });
    }
    Err(if !whole.is_empty() {
        LlmNotRunnable::TooLarge
    } else if split {
        LlmNotRunnable::SeveralFiles
    } else {
        LlmNotRunnable::NoQuant
    })
}

/// The id and folder of `repo`'s model in quantization `quant`: `llm--<owner>--<repository>--<quant>`, lower case.
pub fn id_for(repo: &str, quant: &str) -> String {
    let name: String = format!("{}--{}", repo.replace('/', "--"), quant)
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '-' })
        .collect();
    format!("{PREFIX}{name}")
}

/// A model of the short list: its one GGUF file at the root of `repo`, pinned at `commit` (`cue-cli pin-llm`).
#[allow(clippy::too_many_arguments)]
fn pinned(name: &str, repo: &str, commit: &str, quant: &str, file: &str, size: u64, sha256: &str, license: &str) -> Manifest {
    Manifest {
        id: id_for(repo, quant),
        name: name.to_owned(),
        repo: repo.to_owned(),
        commit: commit.to_owned(),
        files: vec![crate::model::ModelFile { path: file.to_owned(), name: file.to_owned(), size, sha256: sha256.to_owned() }],
        languages: Vec::new(),
        license: license.to_owned(),
    }
}

/// The short list, the recommended model first, pinned at their commits like the speech models. Picked from their
/// model cards (Russian and English, open licenses, no sign-in), not measured: an instruction model without thinking
/// first, then a second family, then the smallest and fastest.
pub fn catalog() -> Vec<Manifest> {
    vec![
        pinned(
            "Qwen3 4B Instruct",
            "unsloth/Qwen3-4B-Instruct-2507-GGUF",
            "a06e946bb6b655725eafa393f4a9745d460374c9",
            "Q4_K_M",
            "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
            2_497_281_120,
            "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597",
            "apache-2.0",
        ),
        pinned(
            "Gemma 4 E2B",
            "unsloth/gemma-4-E2B-it-GGUF",
            "0314792d7f1f7e229411f620751375812bb9faf2",
            "Q4_K_M",
            "gemma-4-E2B-it-Q4_K_M.gguf",
            3_106_738_272,
            "740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8",
            "apache-2.0",
        ),
        pinned(
            "Qwen3 1.7B",
            "unsloth/Qwen3-1.7B-GGUF",
            "d7f544eead698dbd1f15126ef60b45a1e1933222",
            "Q4_K_M",
            "Qwen3-1.7B-Q4_K_M.gguf",
            1_107_409_472,
            "b139949c5bd74937ad8ed8c8cf3d9ffb1e99c866c823204dc42c0d91fa181897",
            "apache-2.0",
        ),
    ]
}

/// The recommended model's id: the short list's first; none while the list is empty.
pub fn default_id() -> Option<String> {
    catalog().first().map(|m| m.id.clone())
}

/// The language model `id`: from the short list, or a found one's manifest in its folder under `models`.
pub fn resolve(models: &Path, id: &str) -> Option<Manifest> {
    catalog()
        .into_iter()
        .find(|m| m.id == id)
        .or_else(|| id.starts_with(PREFIX).then(|| read_manifest(&models.join(id))).flatten().filter(|m| m.id == id))
}

/// The found language models with a folder under `models` (downloaded or on their way), not on the short list, by name.
pub fn found(models: &Path) -> Vec<Manifest> {
    let listed: Vec<String> = catalog().into_iter().map(|m| m.id).collect();
    let mut out: Vec<Manifest> = std::fs::read_dir(models)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            (id.starts_with(PREFIX) && !listed.contains(&id)).then(|| read_manifest(&e.path())).flatten().filter(|m| m.id == id)
        })
        .collect();
    out.sort_by_key(|m| m.name.to_lowercase());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{read_manifest, write_manifest, ModelFile};

    fn files(list: &[(&str, u64)]) -> Vec<RepoFile> {
        list.iter().map(|&(path, size)| RepoFile { path: path.into(), size }).collect()
    }
    fn meta(architecture: Option<&str>, chat_template: bool) -> GgufMeta {
        GgufMeta { architecture: architecture.map(str::to_owned), chat_template }
    }
    fn ok() -> GgufMeta {
        meta(Some("qwen3"), true)
    }
    const GB: u64 = 1_000_000_000;

    #[test]
    fn picks_the_first_quant_in_order() {
        let pick = runnable(&files(&[("x-Q8_0.gguf", 4 * GB), ("x-Q4_K_M.gguf", 2 * GB), ("x-Q5_K_M.gguf", 3 * GB)]), &ok()).unwrap();
        assert_eq!((pick.file.path.as_str(), pick.quant), ("x-Q4_K_M.gguf", "Q4_K_M"));
        let pick = runnable(&files(&[("sub/Model.q4_k_s.gguf", GB), ("Model.Q8_0.gguf", GB)]), &ok()).unwrap();
        assert_eq!((pick.file.path.as_str(), pick.quant), ("sub/Model.q4_k_s.gguf", "Q4_K_S"), "case does not matter");
    }

    #[test]
    fn ignores_projectors_and_non_gguf() {
        let pick = runnable(&files(&[("mmproj-x-f16.gguf", GB), ("README.md", 1), ("x-Q4_0.gguf", GB)]), &ok()).unwrap();
        assert_eq!(pick.quant, "Q4_0");
        assert_eq!(runnable(&files(&[("x-F16.gguf", GB), ("x.safetensors", GB)]), &ok()).unwrap_err(), LlmNotRunnable::NoQuant);
    }

    #[test]
    fn split_models_are_several_files() {
        let split = files(&[("x-Q4_K_M-00001-of-00002.gguf", GB), ("x-Q4_K_M-00002-of-00002.gguf", GB)]);
        assert_eq!(runnable(&split, &ok()).unwrap_err(), LlmNotRunnable::SeveralFiles);
    }

    #[test]
    fn too_large() {
        assert_eq!(runnable(&files(&[("x-Q4_K_M.gguf", MAX_BYTES + 1)]), &ok()).unwrap_err(), LlmNotRunnable::TooLarge);
        let pick = runnable(&files(&[("x-Q4_K_M.gguf", MAX_BYTES + 1), ("x-Q4_0.gguf", MAX_BYTES)]), &ok()).unwrap();
        assert_eq!(pick.quant, "Q4_0", "a smaller quant that fits is taken");
    }

    #[test]
    fn no_template_and_unknown_architecture() {
        let one = files(&[("x-Q4_K_M.gguf", GB)]);
        assert_eq!(runnable(&one, &meta(Some("qwen3"), false)).unwrap_err(), LlmNotRunnable::NoChatTemplate);
        assert_eq!(runnable(&one, &meta(Some("bert"), true)).unwrap_err(), LlmNotRunnable::Architecture);
        assert!(runnable(&one, &meta(None, true)).is_ok(), "an architecture not listed is checked when it loads");
    }

    #[test]
    fn ids_are_lower_case() {
        assert_eq!(id_for("Qwen/Qwen3-4B-GGUF", "Q4_K_M"), "llm--qwen--qwen3-4b-gguf--q4_k_m");
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cue-llm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn manifest(id: &str) -> Manifest {
        Manifest {
            id: id.into(),
            name: "Qwen3 4B".into(),
            repo: "Qwen/Qwen3-4B-GGUF".into(),
            commit: "a".repeat(40),
            files: vec![ModelFile { path: "Qwen3-4B-Q4_K_M.gguf".into(), name: "Qwen3-4B-Q4_K_M.gguf".into(), size: 2 * GB, sha256: "b".repeat(64) }],
            languages: vec![],
            license: "apache-2.0".into(),
        }
    }

    #[test]
    fn found_lists_only_llm_folders() {
        let models = temp("found");
        let id = id_for("Qwen/Qwen3-4B-GGUF", "Q4_K_M");
        write_manifest(&models.join(&id), &manifest(&id)).unwrap();
        write_manifest(&models.join("hf--a--b"), &manifest("hf--a--b")).unwrap();
        write_manifest(&models.join("llm--other"), &manifest("llm--not-this-folder")).unwrap();
        let found: Vec<String> = found(&models).into_iter().map(|m| m.id).collect();
        assert_eq!(found, std::slice::from_ref(&id));
        assert_eq!(resolve(&models, &id).map(|m| m.id), Some(id));
        assert!(resolve(&models, "hf--a--b").is_none(), "a speech model is not a language model");
        assert!(read_manifest(&models.join("hf--a--b")).is_some());
        let _ = std::fs::remove_dir_all(&models);
    }

    #[test]
    fn the_short_list_is_three_with_qwen_recommended() {
        let names: Vec<String> = catalog().into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["Qwen3 4B Instruct", "Gemma 4 E2B", "Qwen3 1.7B"]);
        assert_eq!(default_id().as_deref(), Some("llm--unsloth--qwen3-4b-instruct-2507-gguf--q4_k_m"));
        assert!(catalog().iter().all(|m| m.total() <= MAX_BYTES));
    }

    #[test]
    fn catalog_is_sound() {
        for m in catalog() {
            assert!(m.sound(), "{}", m.id);
            assert_eq!(m.files.len(), 1, "{}", m.id);
            assert!(m.files[0].path.to_lowercase().ends_with(".gguf"), "{}", m.id);
            let pick = runnable(&[RepoFile { path: m.files[0].path.clone(), size: m.files[0].size }], &meta(None, true)).unwrap();
            assert_eq!(m.id, id_for(&m.repo, pick.quant), "{}", m.id);
        }
        assert_eq!(default_id(), catalog().first().map(|m| m.id.clone()));
    }
}

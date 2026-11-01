//! The speech model CUE downloads once (spec "Local speech recognition"): which files, from which commit of which
//! repository, with their sizes and SHA-256; and the model's state as the windows show it.
use serde::Serialize;

/// One file of the model: where it is in the repository, its name in the model's folder, its size and SHA-256.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelFile {
    pub path: &'static str,
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// A model: a Hugging Face repository at a fixed commit (never `main`: the files must be the ones checked here), and
/// the folder it goes into under `models`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub id: &'static str,
    pub name: &'static str,
    pub repo: &'static str,
    pub commit: &'static str,
    pub files: &'static [ModelFile],
}

/// Parakeet Ultra (Moondream's post-trained NVIDIA Parakeet TDT 0.6B v3), int8 ONNX export (CC-BY-4.0).
pub const ULTRA: Manifest = Manifest {
    id: "parakeet-ultra-int8",
    name: "Parakeet Ultra",
    repo: "Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx",
    commit: "b27f93c2238aa35e138996c005a62f1e8430f801",
    files: &[
        ModelFile {
            path: "vocab.txt",
            name: "vocab.txt",
            size: 93_939,
            sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
        },
        ModelFile {
            path: "int8/decoder_joint-model.int8.onnx",
            name: "decoder_joint-model.int8.onnx",
            size: 18_203_490,
            sha256: "667ffde2242a560923cba6bc7f69ef03d736f8a36ca9588f2e6f9fa818cf2fc1",
        },
        ModelFile {
            path: "int8/encoder-model.int8.onnx",
            name: "encoder-model.int8.onnx",
            size: 649_431_593,
            sha256: "714047d70be4b612dce02e0ab2f9e0d6552f78adc421f408d154070796bc2dcf",
        },
    ],
};

impl Manifest {
    pub fn url(&self, f: &ModelFile) -> String {
        format!("https://huggingface.co/{}/resolve/{}/{}", self.repo, self.commit, f.path)
    }

    /// Bytes of every file together.
    pub fn total(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// The model's state, as the Drop, the main window and the welcome window show it (`cue://model`).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_name_the_commit_not_main() {
        let f = &ULTRA.files[2];
        assert_eq!(
            ULTRA.url(f),
            "https://huggingface.co/Olicorne/parakeet-tdt-0.6b-v3-ultra-onnx/resolve/b27f93c2238aa35e138996c005a62f1e8430f801/int8/encoder-model.int8.onnx"
        );
        assert_eq!(ULTRA.total(), 93_939 + 18_203_490 + 649_431_593);
        assert!(ULTRA.files.iter().all(|f| f.sha256.len() == 64 && f.sha256.chars().all(|c| c.is_ascii_hexdigit())));
    }

    #[test]
    fn states_go_to_the_windows_by_name() {
        let json = |s: ModelState| serde_json::to_string(&s).unwrap();
        assert_eq!(json(ModelState::Downloading { done: 1, total: 2 }), r#"{"state":"downloading","done":1,"total":2}"#);
        assert_eq!(json(ModelState::Ready { total: 7 }), r#"{"state":"ready","total":7}"#);
        assert_eq!(json(ModelState::Failed { done: 0, total: 9 }), r#"{"state":"failed","done":0,"total":9}"#);
        assert_eq!(json(ModelState::Unsupported), r#"{"state":"unsupported"}"#);
    }
}

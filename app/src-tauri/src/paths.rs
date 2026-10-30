//! Where CUE keeps its data: settings in `%APPDATA%\CUE`, everything else in `%LOCALAPPDATA%\CUE`.
//! Both folders, settings.json, usage.csv, the history and the bench folder come from the shared helper
//! `cue_core::paths`, which `cue-cli usage` and `cue-bench` use too.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Paths {
    pub settings: PathBuf,
    pub last_dir: PathBuf,
    pub logs: PathBuf,
    pub usage: PathBuf,
    /// The history's day files (spec 3 §2): only the core writes them, the main window reads them.
    pub history: PathBuf,
    pub bench_recordings: PathBuf,
}

impl Paths {
    pub fn new(appdata: &Path, local: &Path) -> Self {
        let local_dir = cue_core::paths::local_dir(local);
        Self {
            settings: cue_core::paths::settings_json(appdata),
            last_dir: local_dir.join("last"),
            logs: local_dir.join("logs"),
            usage: cue_core::paths::usage_csv(local),
            history: cue_core::paths::history_dir(local),
            bench_recordings: cue_core::paths::bench_dir(local).join("recordings"),
        }
    }

    /// Both variables must be set and absolute: otherwise CUE's files would land in the process's current folder.
    pub fn from_env() -> anyhow::Result<Self> {
        let appdata =
            cue_core::paths::app_data().ok_or_else(|| anyhow::anyhow!("no %APPDATA% (or the path is not absolute)"))?;
        let local = cue_core::paths::local_app_data()
            .ok_or_else(|| anyhow::anyhow!("no %LOCALAPPDATA% (or the path is not absolute)"))?;
        Ok(Self::new(&appdata, &local))
    }

    /// Audio of the attempt in progress. A successful dictation moves it to `last_wav`; after a failure it waits
    /// for "retry".
    pub fn current_wav(&self) -> PathBuf {
        self.last_dir.join("current.wav")
    }

    pub fn last_wav(&self) -> PathBuf {
        self.last_dir.join("audio.wav")
    }

    pub fn last_text(&self) -> PathBuf {
        self.last_dir.join("result.txt")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let p = Paths::new(Path::new(r"C:\U\AppData\Roaming"), Path::new(r"C:\U\AppData\Local"));
        assert_eq!(p.settings, Path::new(r"C:\U\AppData\Roaming\CUE\settings.json"));
        assert_eq!(p.last_wav(), Path::new(r"C:\U\AppData\Local\CUE\last\audio.wav"));
        assert_eq!(p.logs, Path::new(r"C:\U\AppData\Local\CUE\logs"));
        assert_eq!(p.usage, Path::new(r"C:\U\AppData\Local\CUE\usage.csv"));
        assert_eq!(p.history, Path::new(r"C:\U\AppData\Local\CUE\history"));
        assert_eq!(p.bench_recordings, Path::new(r"C:\U\AppData\Local\CUE\bench\recordings"));
    }

    #[test]
    fn attempt_files_live_in_last_dir() {
        let p = Paths::new(Path::new(r"C:\U\AppData\Roaming"), Path::new(r"C:\U\AppData\Local"));
        assert_eq!(p.current_wav(), Path::new(r"C:\U\AppData\Local\CUE\last\current.wav"));
        assert_eq!(p.last_text(), Path::new(r"C:\U\AppData\Local\CUE\last\result.txt"));
        // The same file `cue-cli usage` reads.
        assert_eq!(p.usage, cue_core::paths::usage_csv(Path::new(r"C:\U\AppData\Local")));
    }
}

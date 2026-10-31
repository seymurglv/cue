//! Files of the last dictation. The attempt's audio is written to `last\current.wav`;
//! `last\audio.wav` and `last\result.txt` change only on success, so Ctrl+Win+V always pastes
//! the last good result, and "retry" after a failure finds its own audio.
use std::io;
use std::path::Path;

use cue_core::flow::Outcome;

use crate::paths::Paths;

/// What to do with the files when an attempt has ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastFiles {
    /// Text obtained (pasted or left in the clipboard): the attempt's audio and the bare text become the last
    /// dictation.
    Commit,
    /// Failure: the audio stays in `current.wav` for "retry"; the previous dictation is not touched.
    KeepForRetry,
    /// Cancel, empty text, discard, an "Ask" answer: nothing pasted, the attempt's audio is deleted; the previous
    /// result stays.
    Discard,
}

pub fn last_files(outcome: Outcome) -> LastFiles {
    match outcome {
        Outcome::Inserted | Outcome::NoField => LastFiles::Commit,
        Outcome::Failed => LastFiles::KeepForRetry,
        Outcome::Cancelled | Outcome::Empty | Outcome::Discarded | Outcome::Answered => LastFiles::Discard,
    }
}

/// Whose sound `current.wav` holds when a text becomes the last dictation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    /// The attempt's own recording.
    Own,
    /// No attempt's: the attempt's own file could not be made, so whatever is there was left by an earlier one.
    Stale,
    /// Another attempt's, still in use — a follow-up's — or none: a text without audio of its own.
    Other,
}

/// `LastFiles::Commit`: `result.txt` gets the bare text (no join space, line breaks not replaced), and the attempt's
/// own recording (`Sound::Own`) becomes `audio.wav`. Otherwise, and when the attempt has no audio (the WAV was not
/// created), the old `audio.wav` is deleted, so the audio and the text of the last dictation do not diverge: a stale
/// `current.wav` goes too, another attempt's stays.
pub fn commit(paths: &Paths, text: &str, sound: Sound) -> io::Result<()> {
    std::fs::create_dir_all(&paths.last_dir)?;
    // Through a temporary file: an interrupted write will not leave half the text under the name result.txt.
    let tmp = paths.last_dir.join("result.txt.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, paths.last_text())?;
    match sound {
        Sound::Own => match std::fs::rename(paths.current_wav(), paths.last_wav()) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => remove_if_exists(&paths.last_wav()),
            other => other,
        },
        Sound::Stale => remove_if_exists(&paths.current_wav()).and_then(|()| remove_if_exists(&paths.last_wav())),
        Sound::Other => remove_if_exists(&paths.last_wav()),
    }
}

/// `LastFiles::Discard`: delete the attempt's audio, if there is any.
pub fn discard(paths: &Paths) -> io::Result<()> {
    remove_if_exists(&paths.current_wav())
}

/// Delete a file; a missing file is not an error.
fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder per test; removed even when the check failed.
    struct Dir(std::path::PathBuf, Paths);

    impl Dir {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let root = std::env::temp_dir().join(format!("cue-last-{tag}-{}-{nanos}", std::process::id()));
            let paths = Paths::new(&root.join("Roaming"), &root.join("Local"));
            std::fs::create_dir_all(&paths.last_dir).unwrap();
            Self(root, paths)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn read(p: &Path) -> Option<String> {
        std::fs::read_to_string(p).ok()
    }

    #[test]
    fn decision_by_outcome() {
        assert_eq!(last_files(Outcome::Inserted), LastFiles::Commit);
        assert_eq!(last_files(Outcome::NoField), LastFiles::Commit);
        assert_eq!(last_files(Outcome::Failed), LastFiles::KeepForRetry);
        assert_eq!(last_files(Outcome::Cancelled), LastFiles::Discard);
        assert_eq!(last_files(Outcome::Empty), LastFiles::Discard);
        assert_eq!(last_files(Outcome::Discarded), LastFiles::Discard);
        assert_eq!(last_files(Outcome::Answered), LastFiles::Discard);
    }

    #[test]
    fn commit_replaces_last_dictation() {
        let d = Dir::new("commit");
        let p = &d.1;
        std::fs::write(p.last_wav(), "old audio").unwrap();
        std::fs::write(p.last_text(), "старый текст").unwrap();
        std::fs::write(p.current_wav(), "new audio").unwrap();
        commit(p, "Новый текст.\n- пункт", Sound::Own).unwrap();
        assert_eq!(read(&p.last_text()).as_deref(), Some("Новый текст.\n- пункт"), "the bare text as is");
        assert_eq!(read(&p.last_wav()).as_deref(), Some("new audio"));
        assert!(!p.current_wav().exists());
    }

    #[test]
    fn commit_without_new_audio_drops_old_audio() {
        let d = Dir::new("noaudio");
        let p = &d.1;
        std::fs::write(p.last_wav(), "old audio").unwrap();
        commit(p, "текст", Sound::Own).unwrap();
        assert_eq!(read(&p.last_text()).as_deref(), Some("текст"));
        assert!(!p.last_wav().exists(), "the previous dictation's audio must not stay next to the new text");
    }

    /// An attempt whose own file could not be made never takes the sound an earlier attempt left in `current.wav`: it
    /// goes. A text without audio of its own leaves another attempt's where it is.
    #[test]
    fn commit_takes_only_the_attempts_own_sound() {
        let d = Dir::new("sound");
        let p = &d.1;
        std::fs::write(p.last_wav(), "old audio").unwrap();
        std::fs::write(p.current_wav(), "an earlier attempt's audio, kept for its retry").unwrap();
        commit(p, "текст", Sound::Stale).unwrap();
        assert_eq!(read(&p.last_text()).as_deref(), Some("текст"));
        assert!(!p.last_wav().exists() && !p.current_wav().exists());

        std::fs::write(p.last_wav(), "old audio").unwrap();
        std::fs::write(p.current_wav(), "a follow-up's audio").unwrap();
        commit(p, "ответ", Sound::Other).unwrap();
        assert!(!p.last_wav().exists());
        assert_eq!(read(&p.current_wav()).as_deref(), Some("a follow-up's audio"));
        commit(p, "ответ", Sound::Other).unwrap();
    }

    #[test]
    fn commit_creates_last_dir() {
        let d = Dir::new("mkdir");
        let p = &d.1;
        std::fs::remove_dir_all(&p.last_dir).unwrap();
        commit(p, "текст", Sound::Own).unwrap();
        assert_eq!(read(&p.last_text()).as_deref(), Some("текст"));
    }

    #[test]
    fn discard_keeps_previous_dictation() {
        let d = Dir::new("discard");
        let p = &d.1;
        std::fs::write(p.last_wav(), "old audio").unwrap();
        std::fs::write(p.last_text(), "старый текст").unwrap();
        std::fs::write(p.current_wav(), "cancelled audio").unwrap();
        discard(p).unwrap();
        assert!(!p.current_wav().exists());
        assert_eq!(read(&p.last_text()).as_deref(), Some("старый текст"), "Ctrl+Win+V pastes the old result");
        assert_eq!(read(&p.last_wav()).as_deref(), Some("old audio"));
        discard(p).unwrap();
    }
}

//! The cleanup's instructions file, `%APPDATA%\CUE\instructions.md` (`cue_core::instructions`): written with the standard
//! text when missing (or with the text of the old custom style, once), read again when its modified time changes (a saved
//! change is taken at the next dictation, checked at most every 300 ms), cut at 2 000 characters, and replaced by the
//! standard text when it is empty, deleted or not text. CUE keeps the version it last used as `instructions.prev.md`
//! and owns `AGENTS.md` beside the file, for an AI that edits it.
use std::borrow::Cow;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use cue_core::instructions as texts;

use crate::paths::Paths;

/// A changed file is looked for at most this often: an editor saving in several steps is read once.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// What tells a file changed: its modified time and its length.
type Stamp = (SystemTime, u64);

pub struct Instructions {
    file: PathBuf,
    prev: PathBuf,
    /// The interface language: the standard text a missing file, a broken file and "restore" use.
    lang: String,
    /// The text requests use: the file's, trimmed and cut, or the standard one.
    text: String,
    /// The file as it was when read: what `instructions.prev.md` keeps when the file changes. `None`: nothing usable.
    raw: Option<String>,
    stamp: Option<Stamp>,
    checked: Instant,
    edited: bool,
    broken: bool,
    /// How many times the file was read after loading (tests count them).
    reloads: u32,
}

impl Instructions {
    /// Writes the file and `AGENTS.md` when missing (the file with `legacy`, the old custom style's text, when there is
    /// one, else the standard text in `lang`), then reads the file. Never fails: a folder that cannot be written leaves
    /// the standard text in use.
    pub fn load_or_create(paths: &Paths, lang: &str, legacy: Option<&str>) -> Self {
        Self::load_or_create_for(paths, lang, legacy, env!("CARGO_PKG_VERSION"))
    }

    fn load_or_create_for(paths: &Paths, lang: &str, legacy: Option<&str>, version: &str) -> Self {
        let mut me = Self {
            file: paths.instructions(),
            prev: paths.instructions_prev(),
            lang: lang.to_owned(),
            text: texts::standard(lang).to_owned(),
            raw: None,
            stamp: None,
            checked: Instant::now(),
            edited: false,
            broken: false,
            reloads: 0,
        };
        if let Some(dir) = me.file.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            tracing::warn!("instructions: folder not created: {e}");
        }
        write_agents_md(&paths.agents_md(), lang, version);
        if !me.file.exists() {
            let start = legacy.and_then(texts::usable).unwrap_or_else(|| texts::standard(lang));
            match std::fs::write(&me.file, start) {
                Ok(()) => tracing::info!(migrated = legacy.and_then(texts::usable).is_some(), "instructions: file written"),
                Err(e) => tracing::warn!("instructions: file not written: {e}"),
            }
        }
        me.read();
        me.reloads = 0;
        me
    }

    /// Once `instructions.md` exists the old custom text in the settings is no longer needed: clears it and says
    /// whether it did, so the caller writes the settings without it. While the file could not be written the text
    /// stays in settings.json for the next start.
    pub fn settle_legacy(settings: &mut cue_core::settings::Settings, paths: &Paths) -> bool {
        if settings.refine.legacy_custom.is_none() || !paths.instructions().is_file() {
            return false;
        }
        settings.refine.legacy_custom = None;
        true
    }

    /// The instructions for the next request: the file's text, or the standard one while the file is unusable. A file
    /// saved since the last look is read again first (and the version CUE used until now is kept as the previous copy).
    pub fn text(&mut self) -> Cow<'_, str> {
        if self.checked.elapsed() >= DEBOUNCE {
            self.checked = Instant::now();
            if stamp_of(&self.file) != self.stamp {
                self.keep_previous();
                self.read();
            }
        }
        Cow::Borrowed(&self.text)
    }

    /// The file holds instructions other than the standard ones.
    #[allow(dead_code)] // used by the Instructions page
    pub fn edited(&self) -> bool {
        self.edited
    }

    /// The file is empty, deleted or not text: the standard instructions are in use.
    #[allow(dead_code)] // used by the Instructions page
    pub fn broken(&self) -> bool {
        self.broken
    }

    /// Saves `text` as the instructions; the version used until now is kept as `instructions.prev.md` first.
    #[allow(dead_code)] // used by the Instructions page
    pub fn set(&mut self, text: &str) -> io::Result<()> {
        self.keep_previous();
        std::fs::write(&self.file, text)?;
        self.checked = Instant::now();
        self.read();
        Ok(())
    }

    /// "Restore the standard ones": the standard text in `lang` is saved (the previous copy keeps what was there).
    #[allow(dead_code)] // used by the Instructions page
    pub fn reset(&mut self, lang: &str) -> io::Result<()> {
        self.lang = lang.to_owned();
        self.set(texts::standard(lang))
    }

    /// The interface language changed: a missing or broken file now falls back to the standard text in it. The file
    /// itself is not touched.
    pub fn set_lang(&mut self, lang: &str) {
        self.lang = lang.to_owned();
        self.read();
        self.reloads -= 1;
    }

    /// Reads the file into the state.
    fn read(&mut self) {
        self.reloads += 1;
        self.stamp = stamp_of(&self.file);
        let standard = texts::standard(&self.lang);
        let content = std::fs::read(&self.file)
            .ok()
            .and_then(|bytes| (!bytes.contains(&0)).then_some(bytes))
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|s| s.trim_start_matches('\u{feff}').to_owned());
        match content.as_deref().and_then(texts::usable) {
            Some(usable) => {
                self.text = usable.to_owned();
                self.edited = usable != standard.trim() && usable != texts::standard("ru").trim() && usable != texts::standard("en").trim();
                self.broken = false;
                self.raw = content;
            }
            None => {
                self.text = standard.to_owned();
                self.edited = false;
                self.broken = true;
                self.raw = None;
            }
        }
    }

    /// Writes the version in use to `instructions.prev.md`.
    fn keep_previous(&self) {
        if let Some(raw) = &self.raw
            && let Err(e) = std::fs::write(&self.prev, raw)
        {
            tracing::warn!("instructions: previous copy not written: {e}");
        }
    }
}

fn stamp_of(path: &std::path::Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// `AGENTS.md` is CUE's: written when missing or older than `version`, left alone when it says it is newer.
fn write_agents_md(path: &std::path::Path, lang: &str, version: &str) {
    let current = std::fs::read_to_string(path).ok();
    let older = match current.as_deref().and_then(texts::agents_md_version) {
        None => true,
        Some(have) => parse_version(have) < parse_version(version),
    };
    if older && let Err(e) = std::fs::write(path, texts::agents_md(lang, version)) {
        tracing::warn!("instructions: AGENTS.md not written: {e}");
    }
}

fn parse_version(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::refine::{BOLD_PROMPT, BOLD_PROMPT_RU};
    use cue_core::settings::MAX_CUSTOM_CHARS;
    use std::path::Path;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let dir = std::env::temp_dir().join(format!("cue-instr-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn paths(&self) -> Paths {
            Paths::new(&self.0.join("roaming"), &self.0.join("local"))
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn wait() {
        std::thread::sleep(DEBOUNCE + Duration::from_millis(40));
    }

    fn read(p: &Path) -> String {
        std::fs::read_to_string(p).unwrap()
    }

    #[test]
    fn creates_standard_file_when_missing() {
        let d = Dir::new("create");
        let p = d.paths();
        let mut i = Instructions::load_or_create(&p, "ru", None);
        assert_eq!(read(&p.instructions()), BOLD_PROMPT_RU);
        assert_eq!(i.text(), BOLD_PROMPT_RU);
        assert!(!i.edited() && !i.broken());
        assert!(read(&p.agents_md()).starts_with(&format!("<!-- CUE {} -->", env!("CARGO_PKG_VERSION"))));
        let d2 = Dir::new("create-en");
        let mut e = Instructions::load_or_create(&d2.paths(), "en", None);
        assert_eq!(e.text(), BOLD_PROMPT);
    }

    #[test]
    fn migrates_custom_text_once() {
        let d = Dir::new("migrate");
        let p = d.paths();
        let mut i = Instructions::load_or_create(&p, "en", Some("  пиши коротко "));
        assert_eq!(read(&p.instructions()), "пиши коротко");
        assert_eq!((i.text().into_owned(), i.edited()), ("пиши коротко".to_owned(), true));
        // The file exists now: the legacy text is not taken again, and the user's edit stays.
        std::fs::write(p.instructions(), "своё").unwrap();
        let mut again = Instructions::load_or_create(&p, "en", Some("пиши коротко"));
        assert_eq!(again.text(), "своё");
        // A blank legacy text is no text at all.
        let d2 = Dir::new("migrate-blank");
        let mut blank = Instructions::load_or_create(&d2.paths(), "en", Some("  "));
        assert_eq!(blank.text(), BOLD_PROMPT);
    }

    /// The first write of the file fails and settings are saved: the old text is still in settings.json, and the next
    /// start writes the file from it.
    #[test]
    fn a_failed_first_write_keeps_the_old_text_in_settings() {
        use cue_core::settings::{self, Settings};
        let d = Dir::new("legacy-kept");
        let p = d.paths();
        let old = r#"{"refine":{"runs":"local","style":"custom","custom":"пиши коротко"}}"#;
        let sp = p.instructions().with_file_name("settings-test.json");
        std::fs::create_dir_all(p.instructions().parent().unwrap()).unwrap();
        std::fs::write(&sp, old).unwrap();
        let mut s: Settings = settings::load(&sp).unwrap();
        // A folder where the file should be: the write fails.
        std::fs::create_dir_all(p.instructions()).unwrap();
        let _ = Instructions::load_or_create(&p, "en", s.refine.legacy_custom());
        assert!(!Instructions::settle_legacy(&mut s, &p), "no file: the text is kept");
        settings::save(&s, &sp).unwrap();
        let mut next: Settings = settings::load(&sp).unwrap();
        assert_eq!(next.refine.legacy_custom(), Some("пиши коротко"));
        std::fs::remove_dir(p.instructions()).unwrap();
        let _ = Instructions::load_or_create(&p, "en", next.refine.legacy_custom());
        assert_eq!(read(&p.instructions()), "пиши коротко");
        assert!(Instructions::settle_legacy(&mut next, &p));
        assert_eq!(next.refine.legacy_custom(), None);
    }

    #[test]
    fn reloads_after_change() {
        let d = Dir::new("reload");
        let p = d.paths();
        let mut i = Instructions::load_or_create(&p, "en", None);
        wait();
        std::fs::write(p.instructions(), "Keep it short.").unwrap();
        assert_eq!(i.text(), "Keep it short.");
        assert!(i.edited());
        assert_eq!(read(&p.instructions_prev()), BOLD_PROMPT, "the version CUE last used is kept");
    }

    #[test]
    fn burst_of_saves_reloads_once() {
        let d = Dir::new("burst");
        let p = d.paths();
        let mut i = Instructions::load_or_create(&p, "en", None);
        wait();
        assert_eq!(i.text(), BOLD_PROMPT, "looked at, nothing changed");
        for n in 1..=4 {
            std::fs::write(p.instructions(), "x".repeat(n)).unwrap();
            // Asked in the middle of the burst, before the debounce is over: nothing is read yet.
            assert_ne!(i.text(), "xxxx");
        }
        assert_eq!(i.reloads, 0);
        wait();
        assert_eq!(i.text(), "xxxx");
        assert_eq!(i.text(), "xxxx");
        assert_eq!(i.reloads, 1);
    }

    #[test]
    fn empty_or_deleted_or_binary_file_means_standard_and_broken() {
        let d = Dir::new("broken");
        let p = d.paths();
        let mut i = Instructions::load_or_create(&p, "ru", None);
        for content in [Some(Vec::new()), Some(b"  \n ".to_vec()), Some(vec![0xff, 0xfe, 0x00, 0x41]), None] {
            wait();
            match content {
                Some(bytes) => std::fs::write(p.instructions(), bytes).unwrap(),
                None => std::fs::remove_file(p.instructions()).unwrap(),
            }
            assert_eq!(i.text(), BOLD_PROMPT_RU);
            assert!(i.broken() && !i.edited());
        }
        wait();
        std::fs::write(p.instructions(), "Пиши коротко.").unwrap();
        assert_eq!(i.text(), "Пиши коротко.");
        assert!(!i.broken() && i.edited());
    }

    #[test]
    fn oversized_file_is_cut_not_rejected() {
        let d = Dir::new("big");
        let p = d.paths();
        let mut i = Instructions::load_or_create(&p, "en", None);
        wait();
        std::fs::write(p.instructions(), "я".repeat(MAX_CUSTOM_CHARS + 300)).unwrap();
        assert_eq!(i.text().chars().count(), MAX_CUSTOM_CHARS);
        assert!(!i.broken() && i.edited());
    }

    #[test]
    fn set_keeps_previous_copy() {
        let d = Dir::new("set");
        let p = d.paths();
        let mut i = Instructions::load_or_create(&p, "en", None);
        i.set("First.").unwrap();
        assert_eq!(read(&p.instructions_prev()), BOLD_PROMPT);
        i.set("Second.").unwrap();
        assert_eq!((read(&p.instructions()), read(&p.instructions_prev())), ("Second.".to_owned(), "First.".to_owned()));
        assert_eq!(i.text(), "Second.");
        // The change CUE made itself is not noticed again as someone else's.
        wait();
        assert_eq!(i.text(), "Second.");
        assert_eq!(read(&p.instructions_prev()), "First.");
        i.reset("ru").unwrap();
        assert_eq!((read(&p.instructions()), read(&p.instructions_prev())), (BOLD_PROMPT_RU.to_owned(), "Second.".to_owned()));
        assert!(!i.edited());
    }

    #[test]
    fn agents_md_rewritten_when_older_than_version() {
        let d = Dir::new("agents");
        let p = d.paths();
        std::fs::create_dir_all(p.agents_md().parent().unwrap()).unwrap();
        std::fs::write(p.agents_md(), "<!-- CUE 0.9.0 -->\nold").unwrap();
        Instructions::load_or_create_for(&p, "en", None, "0.10.0");
        assert_eq!(read(&p.agents_md()), texts::agents_md("en", "0.10.0"));
        // The same version, or a newer one, is left as it is.
        std::fs::write(p.agents_md(), "<!-- CUE 0.10.0 -->\nedited").unwrap();
        Instructions::load_or_create_for(&p, "en", None, "0.10.0");
        Instructions::load_or_create_for(&p, "en", None, "0.9.0");
        assert_eq!(read(&p.agents_md()), "<!-- CUE 0.10.0 -->\nedited");
        // A file that is not CUE's own is replaced.
        std::fs::write(p.agents_md(), "something").unwrap();
        Instructions::load_or_create_for(&p, "en", None, "0.10.0");
        assert_eq!(read(&p.agents_md()), texts::agents_md("en", "0.10.0"));
    }
}

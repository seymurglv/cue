//! `cue-cli leak-scan <file or folder>...`: looks through files that are to leave this computer (the release
//! program, the installer, the window pages) for anything private, and says where it is: the file, the byte offset and
//! what kind of thing it is, never the thing itself. Any finding makes the exit code 1. A key, a setting, the history,
//! the microphone list, git or a given path that cannot be read, or a given path with no files, makes it 2: a scan that
//! does not know what to look for, or where, would pass blind. A crash prints where it happened, never its message
//! (`main`).
//!
//! What it looks for:
//! - patterns: a path into a Windows user profile (`C:\Users\<name>`, also with `/` or `\\`, URL-encoded, and
//!   `/c/Users/<name>`), an email address, and an Anthropic or OpenRouter key (`sk-ant-` or `sk-or-` and 40 characters
//!   more: the made-up keys of the tests are shorter);
//! - exact matches with what this computer holds, read into memory for the scan only: the keys saved in Credential
//!   Manager, the history texts, questions and texts as heard of the last 31 days (12 characters or longer), the
//!   dictionary terms and the microphone names (4 or longer), the Windows user name, computer name and profile path,
//!   the git user name and email, and the private folders the repository's `.git/info/exclude` names (its folder
//!   lines, 4 characters or longer);
//! - recordings: an Ogg stream whose first page holds `OpusHead` (the history's recordings) and a RIFF `WAVE` file
//!   with its format chunk (the WAVs CUE records, the bench's), at the start of a file or anywhere in one — a file an
//!   installer stores as it is starts there;
//! - in text files only, long random-looking strings: a key of a kind no pattern knows, also one split by `-` or
//!   `_` (a UUID), or by `+` or `/` (base64). Hashes of known kinds (npm integrity, `Cargo.lock` checksums, base64 data
//!   URLs, a commit or hash in a URL path, a Windows GUID in braces) are passed over. Binaries (the program, fonts,
//!   installers) get the other checks only: compressed data and machine code look random throughout. Text is UTF-8
//!   (with a NUL byte now and then), UTF-16LE, or, in any other encoding, mostly printable ASCII.
//!
//! Text is looked for as ASCII or UTF-8 and as UTF-16LE (Windows resources, installer strings), ASCII letters in any
//! case. [`PUBLIC_NAME`] is the one name CUE publishes: when it is also the Windows user name or the computer name, it
//! is a finding only next to `\`, `/` or `@` (a path or an address), not in words, and not as the installer's registry
//! key `Software\<publisher>`, followed by `\CUE` or by a character no path holds.
use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use aho_corasick::AhoCorasick;
use anyhow::{Context, Result, anyhow, bail};
use cue_core::history;
use cue_engines::keys::{self, KeyName};
use regex::bytes::Regex;

/// The copyright holder in NOTICE: the one name CUE publishes.
pub const PUBLIC_NAME: &str = "seymurglv";
/// The history days looked for, counted back from today.
const HISTORY_DAYS: u32 = 31;
/// A history text shorter than this is a common phrase rather than anything of the user's.
const MIN_HISTORY_CHARS: usize = 12;
/// A shorter dictionary term or name ("AI", "Mic") is a part of too many ordinary words.
const MIN_NAME_CHARS: usize = 4;
/// Profile folders that are nobody's: those of Windows itself and of the build services that compile third-party
/// tools such as the installer's plugins.
const COMMON_PROFILES: [&str; 6] =
    ["public", "default", "default user", "all users", "runneradmin", "vssadministrator"];
/// Findings printed for one file; the rest are counted.
const MAX_SHOWN: usize = 50;

/// What a finding is. Its text names the kind only: a saved key by its name, a dictionary term by its number in the
/// settings file, a private folder by its number among the folder lines of `.git/info/exclude`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    ProfilePath,
    UserName,
    Email,
    KeyLike,
    ComputerName,
    PrivateFolder(usize),
    SavedKey(&'static str),
    HistoryText,
    DictionaryTerm(usize),
    MicrophoneName,
    GitName,
    GitEmail,
    RandomString,
    /// A recording, by its format.
    Recording(&'static str),
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::ProfilePath => f.write_str("a path into a user profile"),
            Kind::UserName => f.write_str("the Windows user name"),
            Kind::Email => f.write_str("an email address"),
            Kind::KeyLike => f.write_str("an API key (sk-ant- or sk-or-)"),
            Kind::ComputerName => f.write_str("the computer name"),
            Kind::PrivateFolder(i) => write!(f, "private folder {} of .git/info/exclude", i + 1),
            Kind::SavedKey(name) => write!(f, "the saved {name} key"),
            Kind::HistoryText => f.write_str("a history text"),
            Kind::DictionaryTerm(i) => write!(f, "dictionary term {}", i + 1),
            Kind::MicrophoneName => f.write_str("a microphone name"),
            Kind::GitName => f.write_str("the git user name"),
            Kind::GitEmail => f.write_str("the git email"),
            Kind::RandomString => f.write_str("a random-looking string"),
            Kind::Recording(format) => write!(f, "a recording ({format})"),
        }
    }
}

/// One finding: where it starts in its file, and what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hit {
    pub offset: usize,
    pub kind: Kind,
}

/// What this computer holds and a build must not. Kept in memory for the scan only; `Debug` shows how many of each.
#[derive(Default)]
pub struct Private {
    exact: Vec<(Kind, String)>,
    user_name: Option<String>,
    /// `None` when it is the user name too: looked for as that.
    computer_name: Option<String>,
    /// [`PUBLIC_NAME`] when gathered.
    public_name: String,
}

impl fmt::Debug for Private {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.summary())
    }
}

impl Private {
    /// Adds a text to look for, trimmed, if it has at least `min_chars` characters.
    fn add(&mut self, kind: Kind, text: &str, min_chars: usize) {
        let text = text.trim();
        if text.chars().count() >= min_chars.max(1) {
            self.exact.push((kind, text.to_owned()));
        }
    }

    fn count(&self, pick: impl Fn(&Kind) -> bool) -> usize {
        self.exact.iter().filter(|(kind, _)| pick(kind)).count()
    }

    /// How many things of each kind are looked for: counts only.
    pub fn summary(&self) -> String {
        let yes_no = |b: bool| if b { "yes" } else { "no" };
        format!(
            "saved keys {}, history texts {}, dictionary terms {}, microphone names {}, user name {}, computer name \
             {}, profile path {}, git name {}, git email {}, private folders {}",
            self.count(|k| matches!(k, Kind::SavedKey(_))),
            self.count(|k| *k == Kind::HistoryText),
            self.count(|k| matches!(k, Kind::DictionaryTerm(_))),
            self.count(|k| *k == Kind::MicrophoneName),
            yes_no(self.user_name.is_some()),
            yes_no(self.computer_name.is_some()),
            yes_no(self.count(|k| *k == Kind::ProfilePath) > 0),
            yes_no(self.count(|k| *k == Kind::GitName) > 0),
            yes_no(self.count(|k| *k == Kind::GitEmail) > 0),
            self.count(|k| matches!(k, Kind::PrivateFolder(_))),
        )
    }

    /// Reads what this computer holds. Every error text names the store or the file, never a value: the settings
    /// file's own parse error is left out, since it can quote a value.
    pub fn gather() -> Result<Private> {
        let mut private = Private { public_name: PUBLIC_NAME.to_owned(), ..Private::default() };
        for name in KeyName::ALL {
            match keys::lookup(name) {
                Ok(Some(value)) => private.add(Kind::SavedKey(name.as_str()), &value, 1),
                Ok(None) => {}
                Err(e) => bail!("could not read the saved keys: {e}"),
            }
        }

        let local = cue_core::paths::local_app_data().context("LOCALAPPDATA is not set or is not an absolute path")?;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
        let stamp = cue_core::usage::utc_timestamp(now);
        for text in history_texts(&cue_core::paths::history_dir(&local), &stamp[..10])? {
            private.add(Kind::HistoryText, &text, MIN_HISTORY_CHARS);
        }

        let roaming = cue_core::paths::app_data().context("APPDATA is not set or is not an absolute path")?;
        let settings = cue_core::settings::load(&cue_core::paths::settings_json(&roaming))
            .map_err(|_| anyhow!("could not read settings.json, which holds the dictionary and the microphone"))?;
        for (i, term) in settings.dictionary.terms.iter().enumerate() {
            private.add(Kind::DictionaryTerm(i), term, MIN_NAME_CHARS);
        }
        let mut microphones: Vec<String> = settings.audio.device.into_iter().collect();
        microphones.extend(cue_platform_win::mic::listed_names().context("Windows did not list the microphones")?);
        microphones.extend(cue_platform_win::mic::default_name());
        microphones.sort();
        microphones.dedup();
        for name in &microphones {
            private.add(Kind::MicrophoneName, name, MIN_NAME_CHARS);
        }

        let user = std::env::var("USERNAME").context("USERNAME is not set")?;
        private.user_name = Some(user.trim().to_owned()).filter(|u| !u.is_empty());
        let computer = std::env::var("COMPUTERNAME").context("COMPUTERNAME is not set")?;
        let same = |c: &str| private.user_name.as_ref().is_some_and(|u| u.eq_ignore_ascii_case(c));
        private.computer_name = Some(computer.trim().to_owned()).filter(|c| !c.is_empty() && !same(c));
        let profile = std::env::var("USERPROFILE").context("USERPROFILE is not set")?;
        for form in profile_forms(profile.trim().trim_end_matches(['\\', '/'])) {
            private.add(Kind::ProfilePath, &form, 1);
        }

        let (Ok(name), Ok(email)) = (git_config("user.name"), git_config("user.email")) else {
            bail!("git did not start, so the git name and email could not be read");
        };
        if let Some(name) = name.filter(|n| !n.eq_ignore_ascii_case(PUBLIC_NAME)) {
            private.add(Kind::GitName, &name, MIN_NAME_CHARS);
        }
        if let Some(email) = email {
            private.add(Kind::GitEmail, &email, 1);
        }

        for (i, folder) in private_folders(&read_exclude()?).iter().enumerate() {
            private.add(Kind::PrivateFolder(i), folder, MIN_NAME_CHARS);
        }
        Ok(private)
    }
}

/// The profile path as it is, with `/` or `\\` for `\`, and URL-encoded: `%5C` or `%2F` for `\`, the colon as it is or
/// as `%3A`.
fn profile_forms(profile: &str) -> Vec<String> {
    let mut forms = vec![profile.to_owned(), profile.replace('\\', "/"), profile.replace('\\', r"\\")];
    for colon in [":", "%3A"] {
        for slash in ["%5C", "%2F"] {
            forms.push(profile.replace(':', colon).replace('\\', slash));
        }
    }
    forms
}

/// The repository's `.git/info/exclude`, where git keeps it (`git rev-parse --git-path`); a missing file names no
/// folder. Errors name the file, never a line of it.
fn read_exclude() -> Result<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()
        .context("git did not start, so .git/info/exclude could not be found")?;
    if !out.status.success() {
        bail!("not in a git repository: leak-scan runs in CUE's, whose .git/info/exclude names its private folders");
    }
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    match std::fs::read(&path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(_) => bail!("could not read .git/info/exclude"),
    }
}

/// The folders an exclude file names: its lines that end with `/` (a folder only), without the slashes around them.
/// Comments, blank lines, negations and patterns with wildcards or escapes name none.
fn private_folders(exclude: &str) -> Vec<String> {
    exclude
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with(['#', '!']) && line.ends_with('/'))
        .map(|line| line.trim_matches('/'))
        .filter(|name| !name.is_empty() && !name.contains(['*', '?', '[', '\\']))
        .map(str::to_owned)
        .collect()
}

/// `git config --get <key>`: `None` when it is not set.
fn git_config(key: &str) -> std::io::Result<Option<String>> {
    let out = std::process::Command::new("git").args(["config", "--get", key]).output()?;
    let value = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    Ok(Some(value).filter(|v| out.status.success() && !v.is_empty()))
}

/// The texts, questions and texts as heard (before CUE cleaned them up) of the history days from `HISTORY_DAYS` days
/// before `today` (UTC, `YYYY-MM-DD`) on, and every line of those with several. One day more is read: the day files are
/// named by the local date. No folder is no history; a folder or a day file that cannot be read is an error.
fn history_texts(dir: &Path, today: &str) -> Result<Vec<String>> {
    let Some(first) = history::date_minus(today, HISTORY_DAYS + 1) else { return Ok(Vec::new()) };
    readable_days(dir, &first)?;
    let mut texts = Vec::new();
    for day in history::older(dir, "9999-12-31", 2 * HISTORY_DAYS as usize).days {
        if day.date < first {
            continue;
        }
        for entry in day.entries {
            for text in [Some(entry.text), entry.question, entry.raw].into_iter().flatten() {
                if text.contains('\n') {
                    texts.extend(text.lines().map(str::to_owned));
                }
                texts.push(text);
            }
        }
    }
    Ok(texts)
}

/// Whether the history folder can be listed and its day files from `first` on read: the history reader passes over
/// what it cannot read, and those days would be looked for blind.
fn readable_days(dir: &Path, first: &str) -> Result<()> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => bail!("could not list the history folder"),
    };
    for entry in listing {
        let entry = entry.map_err(|_| anyhow!("could not list the history folder"))?;
        let name = entry.file_name();
        let date = name.to_str().and_then(|n| n.strip_suffix(".jsonl")).filter(|d| d.len() == 10);
        if !date.is_some_and(|d| d >= first) || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        match std::fs::read(entry.path()) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("could not read a history day file"),
        }
    }
    Ok(())
}

/// Finds what [`Private`] holds and the patterns in the bytes of one file.
pub struct Scanner {
    exact: Option<AhoCorasick>,
    /// The kind of each pattern of `exact`: every text is there twice, as UTF-8 and as UTF-16LE.
    exact_kinds: Vec<Kind>,
    /// The user name and the computer name: each as UTF-8 (pattern 0) and UTF-16LE (pattern 1), whether it is the
    /// public name, and its kind.
    names: Vec<(AhoCorasick, bool, Kind)>,
    profile: Regex,
    patterns: Vec<(Regex, Kind)>,
}

impl Scanner {
    pub fn new(private: &Private) -> Result<Scanner> {
        let mut texts: Vec<Vec<u8>> = Vec::new();
        let mut exact_kinds = Vec::new();
        for (kind, text) in &private.exact {
            texts.push(text.as_bytes().to_vec());
            texts.push(utf16le(text));
            exact_kinds.extend([*kind, *kind]);
        }
        let matcher = |texts: &[Vec<u8>]| AhoCorasick::builder().ascii_case_insensitive(true).build(texts);
        let exact = if texts.is_empty() { None } else { Some(matcher(&texts)?) };
        let mut names = Vec::new();
        for (name, kind) in [(&private.user_name, Kind::UserName), (&private.computer_name, Kind::ComputerName)] {
            if let Some(name) = name {
                let public = name.eq_ignore_ascii_case(&private.public_name);
                names.push((matcher(&[name.as_bytes().to_vec(), utf16le(name)])?, public, kind));
            }
        }
        let pattern = |p: &str| Regex::new(p).expect("a fixed pattern");
        // After the drive (its colon as it is or URL-encoded) and after `Users`: a slash, one or two backslashes, or
        // either URL-encoded.
        let sep = r"(?:\\\\?|/|%5c|%2f)";
        Ok(Scanner {
            exact,
            exact_kinds,
            names,
            profile: pattern(&format!(
                r#"(?i-u)(?:\b[a-z](?::|%3a){sep}|/[a-z]/)users{sep}([^\\/:*?"<>|\x00-\x1f]{{1,64}})"#
            )),
            patterns: vec![
                (pattern(r"(?i-u)[a-z0-9._%+-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)*\.[a-z]{2,24}\b"), Kind::Email),
                (pattern(r"(?i-u)sk-(?:ant|or)-[a-z0-9_-]{40,}"), Kind::KeyLike),
            ],
        })
    }

    /// Every finding in one file's bytes, by offset.
    pub fn scan(&self, bytes: &[u8]) -> Vec<Hit> {
        let mut hits = BTreeSet::new();
        if let Some(exact) = &self.exact {
            for m in exact.find_overlapping_iter(bytes) {
                hits.insert(Hit { offset: m.start(), kind: self.exact_kinds[m.pattern().as_usize()] });
            }
        }
        for (name, public, kind) in &self.names {
            for m in name.find_overlapping_iter(bytes) {
                let wide = m.pattern().as_usize() == 1;
                if !public || touches_a_path(bytes, m.start(), m.end(), wide) {
                    hits.insert(Hit { offset: m.start(), kind: *kind });
                }
            }
        }
        for view in [View::narrow(bytes), View::wide(bytes, 0), View::wide(bytes, 1)] {
            for c in self.profile.captures_iter(&view.bytes) {
                let name = String::from_utf8_lossy(&c[1]).to_ascii_lowercase();
                // A URL-encoded path goes on after the name with its encoded slash.
                let cut = ["%5c", "%2f"].into_iter().filter_map(|slash| name.find(slash)).min();
                let name = name[..cut.unwrap_or(name.len())].trim();
                if !COMMON_PROFILES.contains(&name) && !name.starts_with(['%', '$']) {
                    let start = c.get(0).map_or(0, |m| m.start());
                    hits.insert(Hit { offset: view.offset(start), kind: Kind::ProfilePath });
                }
            }
            for (pattern, kind) in &self.patterns {
                for m in pattern.find_iter(&view.bytes) {
                    hits.insert(Hit { offset: view.offset(m.start()), kind: *kind });
                }
            }
        }
        for (offset, format) in recordings(bytes) {
            hits.insert(Hit { offset, kind: Kind::Recording(format) });
        }
        if let Some(view) = View::text(bytes) {
            for at in random_strings(&view.bytes) {
                hits.insert(Hit { offset: view.offset(at), kind: Kind::RandomString });
            }
        }
        hits.into_iter().collect()
    }
}

/// Where recordings start in `bytes`, and in which format: an Ogg page (RFC 3533, version 0) whose first packet is an
/// `OpusHead` (RFC 7845), and `RIFF`, its size, `WAVE` and right after it a format chunk that reads as one — of 16, 18
/// or 40 bytes, PCM, float, A-law, µ-law or extensible, 1 to 8 channels — whatever the size says: a recording cut off
/// by a crash still holds 0 there. A program's table of these names alone is no recording.
fn recordings(bytes: &[u8]) -> Vec<(usize, &'static str)> {
    let starts = |tag: &'static [u8]| bytes.windows(4).enumerate().filter(move |(_, w)| *w == tag).map(|(at, _)| at);
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let mut found = Vec::new();
    for at in starts(b"OggS") {
        let first = bytes.get(at + 26).map(|&segments| at + 27 + usize::from(segments));
        if bytes.get(at + 4) == Some(&0) && first.is_some_and(|p| bytes.get(p..p + 8) == Some(b"OpusHead")) {
            found.push((at, "Ogg Opus"));
        }
    }
    for at in starts(b"RIFF") {
        let wave = bytes.get(at + 8..at + 16) == Some(b"WAVEfmt ");
        let chunk = u32_at(at + 16).is_some_and(|n| matches!(n, 16 | 18 | 40));
        let format = u16_at(at + 20).is_some_and(|f| matches!(f, 1 | 3 | 6 | 7 | 0xFFFE));
        let channels = u16_at(at + 22).is_some_and(|c| (1..=8).contains(&c));
        if wave && chunk && format && channels {
            found.push((at, "WAV"));
        }
    }
    found.sort_unstable();
    found
}

fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// Whether the character before or after `bytes[start..end]` is `\`, `/` or `@`: the name is a part of a path or an
/// address. The registry key the installer writes under its publisher's name is no path: `Software\` before the name,
/// and after it a character no path holds (a quote, a control character, the end) or `\CUE` and then `\` or such a
/// character. `wide`: the match is UTF-16LE.
fn touches_a_path(bytes: &[u8], start: usize, end: usize, wide: bool) -> bool {
    let as_matched = |text: &[u8]| -> Vec<u8> {
        if wide { text.iter().flat_map(|&c| [c, 0]).collect() } else { text.to_vec() }
    };
    let (key, product) = (as_matched(br"software\"), as_matched(br"\cue"));
    let holds = |at: usize, text: &[u8]| bytes.get(at..at + text.len()).is_some_and(|b| b.eq_ignore_ascii_case(text));
    // The character at `at`, one byte or one UTF-16 unit: none, or one no path holds.
    let path_ends = |at: usize| {
        let c = if wide {
            bytes.get(at..at + 2).map(|u| u32::from(u16::from_le_bytes([u[0], u[1]])))
        } else {
            bytes.get(at).map(|&c| u32::from(c))
        };
        c.is_none_or(|c| c < 0x20 || b"\"<>|*?".iter().any(|&x| u32::from(x) == c))
    };
    if start >= key.len() && holds(start - key.len(), &key) {
        let after = end + product.len();
        let below = holds(end, &product) && (path_ends(after) || holds(after, &as_matched(br"\")));
        if path_ends(end) || below {
            return false;
        }
    }
    let (before, after) = if wide {
        let unit = |at: usize| bytes.get(at..at + 2).filter(|u| u[1] == 0).map(|u| u[0]);
        (start.checked_sub(2).and_then(unit), unit(end))
    } else {
        (start.checked_sub(1).map(|at| bytes[at]), bytes.get(end).copied())
    };
    [before, after].into_iter().flatten().any(|c| matches!(c, b'\\' | b'/' | b'@'))
}

/// The bytes the patterns run over, and how an offset in them maps back to the file.
struct View<'a> {
    bytes: std::borrow::Cow<'a, [u8]>,
    base: usize,
    stride: usize,
}

impl<'a> View<'a> {
    /// The file as it is: ASCII and UTF-8 text.
    fn narrow(bytes: &'a [u8]) -> View<'a> {
        View { bytes: bytes.into(), base: 0, stride: 1 }
    }

    /// The printable ASCII of UTF-16LE text starting at `align` (0 or 1), one byte per character; anything else
    /// becomes a NUL, so no pattern runs across it.
    fn wide(bytes: &[u8], align: usize) -> View<'a> {
        let printable = |c: u8| c.is_ascii_graphic() || matches!(c, b' ' | b'\t' | b'\r' | b'\n');
        let narrowed = bytes
            .get(align..)
            .unwrap_or_default()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&[lo, hi]| if hi == 0 && printable(lo) { lo } else { 0 })
            .collect::<Vec<u8>>();
        View { bytes: narrowed.into(), base: align, stride: 2 }
    }

    /// The view the random-string check reads, if the file is text: UTF-16LE with its byte order mark, UTF-8 with a
    /// NUL byte now and then at most (one a KiB), text in another encoding (a code page) or with a few more NULs —
    /// printable ASCII but for one byte in 20 at most — or UTF-16LE without the mark and without control characters.
    /// Machine code and compressed data are far from either.
    fn text(bytes: &'a [u8]) -> Option<View<'a>> {
        let nuls = bytes.iter().filter(|&&b| b == 0).count();
        let utf8 = nuls <= (bytes.len() / 1024).max(1) && std::str::from_utf8(bytes).is_ok();
        let printable = |b: &&u8| b.is_ascii_graphic() || matches!(**b, b' ' | b'\t' | b'\r' | b'\n');
        let ascii = !bytes.is_empty() && 20 * bytes.iter().filter(printable).count() >= 19 * bytes.len();
        if bytes.starts_with(&[0xFF, 0xFE]) {
            Some(View::wide(bytes, 0))
        } else if utf8 || ascii {
            Some(View::narrow(bytes))
        } else if utf16_text(bytes) {
            Some(View::wide(bytes, 0))
        } else {
            None
        }
    }

    fn offset(&self, at: usize) -> usize {
        self.base + self.stride * at
    }
}

/// Whether `bytes` read as UTF-16LE text: whole code units, no lone surrogate, and no control character but a tab and
/// line breaks — and one NUL unit at the end, which ends a string. Machine code and compressed data hold a NUL unit or a
/// control character within a few bytes.
fn utf16_text(bytes: &[u8]) -> bool {
    let (units, rest) = bytes.as_chunks::<2>();
    let units = units.strip_suffix(&[[0, 0]]).unwrap_or(units);
    !units.is_empty()
        && rest.is_empty()
        && char::decode_utf16(units.iter().map(|&u| u16::from_le_bytes(u)))
            .all(|c| c.is_ok_and(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r')))
}

/// After these, a hash or encoded data of a known kind runs on and is passed over: base64 data URLs, npm integrity,
/// `Cargo.lock` checksums.
const KNOWN_HASHES: [&[u8]; 6] = [b"base64,", b"sha512-", b"sha384-", b"sha256-", b"sha1-", b"checksum = \""];

/// Where random-looking runs of letters and digits start in a text: 24 or more of them with digits and both cases, or
/// 32 or more hex digits with letters among them, with an even enough spread of characters. Runs joined by single `-`
/// or `_` (a UUID, a token in parts) are judged together first, then each alone: the words of a file name or an
/// identifier (`Montserrat-ExtraBold-latin`) change between capitals, small letters and digits too seldom. Runs joined
/// by `+` or `/` too (base64), once they make 32 characters together: shorter ones are a path or arithmetic.
fn random_strings(text: &[u8]) -> Vec<usize> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let known = |p: &&&[u8]| text[i..].get(..p.len()).is_some_and(|t| t.eq_ignore_ascii_case(p));
        if let Some(prefix) = KNOWN_HASHES.iter().find(known) {
            i += prefix.len();
            while i < text.len() && (text[i].is_ascii_alphanumeric() || matches!(text[i], b'+' | b'/' | b'=')) {
                i += 1;
            }
            continue;
        }
        if !text[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let mut runs = Vec::new();
        let mut slashed = false;
        loop {
            let start = i;
            while i < text.len() && text[i].is_ascii_alphanumeric() {
                i += 1;
            }
            runs.push(start..i);
            let joined = text.get(i + 1).is_some_and(u8::is_ascii_alphanumeric);
            if !joined || !matches!(text[i], b'-' | b'_' | b'+' | b'/') {
                break;
            }
            slashed |= matches!(text[i], b'+' | b'/');
            i += 1;
        }
        let start = runs[0].start;
        if runs.len() > 1 {
            let whole: Vec<u8> = runs.iter().flat_map(|r| &text[r.clone()]).copied().collect();
            let long_enough = !slashed || whole.len() >= 32;
            if long_enough && !braced_guid(text, &runs) && !url_hash(text, start, &whole) && random_looking(&whole) {
                found.push(start);
                continue;
            }
        }
        for run in runs {
            if !url_hash(text, run.start, &text[run.clone()]) && random_looking(&text[run.clone()]) {
                found.push(run.start);
            }
        }
    }
    found
}

/// Whether `runs` make a Windows GUID in braces, `{8-4-4-4-12 hex digits}`: the identifier of a COM class or an app.
fn braced_guid(text: &[u8], runs: &[std::ops::Range<usize>]) -> bool {
    let (Some(first), Some(last)) = (runs.first(), runs.last()) else { return false };
    let shape = runs.iter().map(|r| r.len()).eq([8, 4, 4, 4, 12]);
    let hex = runs.iter().all(|r| text[r.clone()].iter().all(u8::is_ascii_hexdigit));
    let dashes = runs.windows(2).all(|w| text[w[0].end] == b'-');
    let braced = first.start > 0 && text[first.start - 1] == b'{' && text.get(last.end) == Some(&b'}');
    shape && hex && dashes && braced
}

/// Whether `run`, at `start` of `text`, is a gist id, a commit or a SHA-256 (32, 40 or 64 hex digits) that makes up a
/// segment of the path of a URL on its line.
fn url_hash(text: &[u8], start: usize, run: &[u8]) -> bool {
    let line = &text[text[..start].iter().rposition(|&c| c == b'\n').map_or(0, |at| at + 1)..start];
    let in_a_url = line.ends_with(b"/") && line.windows(3).any(|w| w == b"://");
    in_a_url && matches!(run.len(), 32 | 40 | 64) && run.iter().all(u8::is_ascii_hexdigit)
}

fn random_looking(run: &[u8]) -> bool {
    if !run.iter().any(u8::is_ascii_digit) || !run.iter().any(u8::is_ascii_alphabetic) || alphabet(run) {
        return false;
    }
    if run.iter().all(u8::is_ascii_hexdigit) {
        return run.len() >= 32 && entropy(run) >= 3.0;
    }
    let both_cases = run.iter().any(u8::is_ascii_uppercase) && run.iter().any(u8::is_ascii_lowercase);
    run.len() >= 24 && both_cases && entropy(run) >= 3.8 && 20 * class_changes(run) >= 9 * (run.len() - 1)
}

/// How many neighbours are of different classes (capital, small letter, digit): about 3 pairs in 5 in random letters
/// and digits, 1 in 3 or fewer in words written together (`MicrosoftEdgeWebView2RuntimeInstaller`), whose every
/// capital starts and ends a change. The bar is 45%.
fn class_changes(run: &[u8]) -> usize {
    let class = |c: u8| (c.is_ascii_uppercase(), c.is_ascii_digit());
    run.windows(2).filter(|w| class(w[0]) != class(w[1])).count()
}

/// A run of mostly consecutive characters, like the digits and letters of an encoding table.
fn alphabet(run: &[u8]) -> bool {
    2 * run.windows(2).filter(|w| w[1] == w[0].wrapping_add(1)).count() >= run.len()
}

/// Shannon entropy in bits per character.
fn entropy(run: &[u8]) -> f64 {
    let mut counts = [0u32; 256];
    for &b in run {
        counts[usize::from(b)] += 1;
    }
    let n = run.len() as f64;
    counts.iter().filter(|&&c| c > 0).map(|&c| f64::from(c) / n).map(|p| -p * p.log2()).sum()
}

/// The files under the given paths, folders walked in name order; links are not followed.
fn files(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let meta = std::fs::symlink_metadata(path).with_context(|| format!("could not read {}", path.display()))?;
    if meta.is_dir() {
        let mut entries = std::fs::read_dir(path)
            .and_then(|listing| listing.map(|e| e.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>())
            .with_context(|| format!("could not list {}", path.display()))?;
        entries.sort();
        for entry in entries {
            files(&entry, out)?;
        }
    } else if meta.is_file() {
        out.push(path.to_owned());
    }
    Ok(())
}

/// `cue-cli leak-scan`: the exit code (0 nothing found, 1 findings, 2 the scan could not run).
pub fn run(paths: &[&str]) -> i32 {
    match scan_paths(paths) {
        Ok(0) => 0,
        Ok(_) => 1,
        Err(e) => {
            eprintln!("leak-scan: {e:#}");
            2
        }
    }
}

/// The files under each of `paths`: a path with none is an error, since nothing would be scanned there.
fn all_files(paths: &[&str]) -> Result<Vec<PathBuf>> {
    let mut all = Vec::new();
    for path in paths {
        let before = all.len();
        files(Path::new(path), &mut all)?;
        if all.len() == before {
            bail!("no files under {path}");
        }
    }
    Ok(all)
}

fn scan_paths(paths: &[&str]) -> Result<usize> {
    let all = all_files(paths)?;
    let private = Private::gather()?;
    eprintln!("leak-scan: looking for {}", private.summary());
    let scanner = Scanner::new(&private)?;
    drop(private);
    let mut total = 0;
    for file in &all {
        let bytes = std::fs::read(file).with_context(|| format!("could not read {}", file.display()))?;
        let hits = scanner.scan(&bytes);
        for hit in hits.iter().take(MAX_SHOWN) {
            println!("{}  offset {}  {}", file.display(), hit.offset, hit.kind);
        }
        if hits.len() > MAX_SHOWN {
            println!("{}  and {} more", file.display(), hits.len() - MAX_SHOWN);
        }
        total += hits.len();
    }
    let result = if total == 0 { "nothing private found".to_owned() } else { format!("{total} findings") };
    println!("leak-scan: {} files, {result}", all.len());
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Letters and digits from a fixed seed: a made-up "key" without a literal one in the source.
    fn made_up(seed: u64, len: usize, alphabet: &[u8]) -> String {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
                char::from(alphabet[(x >> 33) as usize % alphabet.len()])
            })
            .collect()
    }

    const BASE62: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const HEX: &[u8] = b"0123456789abcdef";

    /// A made-up public name: the tests put it in paths, where the real one never is.
    const PUBLIC: &str = "pubname";

    fn scanner(exact: &[(Kind, &str)], user: Option<&str>) -> Scanner {
        scanner_with_computer(exact, user, None)
    }

    fn scanner_with_computer(exact: &[(Kind, &str)], user: Option<&str>, computer: Option<&str>) -> Scanner {
        let mut private = Private { public_name: PUBLIC.to_owned(), ..Private::default() };
        for (kind, text) in exact {
            private.add(*kind, text, 1);
        }
        private.user_name = user.map(str::to_owned);
        private.computer_name = computer.map(str::to_owned);
        Scanner::new(&private).unwrap()
    }

    fn kinds(hits: &[Hit]) -> Vec<Kind> {
        hits.iter().map(|h| h.kind).collect()
    }

    fn offsets(hits: &[Hit], kind: Kind) -> Vec<usize> {
        hits.iter().filter(|h| h.kind == kind).map(|h| h.offset).collect()
    }

    /// Where each of `parts` starts in their concatenation.
    fn starts(parts: &[&str]) -> Vec<usize> {
        parts.iter().scan(0, |at, p| Some(std::mem::replace(at, *at + p.len()))).collect()
    }

    #[test]
    fn profile_paths_are_found_in_every_form() {
        let s = scanner(&[], None);
        let parts = [r"C:\Users\jane\x | ", "c:/users/jane/y | ", r"C:\\Users\\jane | ", "/c/Users/jane/z | "];
        let mut file = parts.concat().into_bytes();
        let wide_at = file.len();
        file.extend(utf16le(r"D:\Users\jane\AppData"));
        let mut expected = starts(&parts);
        expected.push(wide_at);
        assert_eq!(offsets(&s.scan(&file), Kind::ProfilePath), expected);
        // A path is a path in a binary too.
        file.push(0);
        assert_eq!(offsets(&s.scan(&file), Kind::ProfilePath), expected);
    }

    /// In a URL a path's backslashes or slashes, and the drive's colon, may be encoded.
    #[test]
    fn url_encoded_profile_paths_are_found() {
        let s = scanner(&[], None);
        let parts = ["C%3A%5CUsers%5Cjane%5Cx | ", "file:///c:%2fusers%2fJane | ", "D:%5cUsers%5cjane | "];
        let file = [&parts[..], &[r"C%3A%5CUsers%5CPublic%5CDocuments"]].concat().concat();
        let at = starts(&parts);
        assert_eq!(offsets(&s.scan(file.as_bytes()), Kind::ProfilePath), [at[0], at[1] + 8, at[2]]);
        let forms = profile_forms(r"D:\Profiles\jdoe");
        let expected = [r"D:\Profiles\jdoe", "D:/Profiles/jdoe", r"D:\\Profiles\\jdoe", "D%3A%5CProfiles%5Cjdoe"];
        for form in expected.into_iter().chain(["D:%2FProfiles%2Fjdoe"]) {
            assert!(forms.iter().any(|f| f == form), "{form}");
        }
    }

    #[test]
    fn common_profiles_and_placeholders_are_nobodys() {
        let s = scanner(&[], None);
        let file = [
            r"C:\Users\Public\Documents",
            r"C:\Users\runneradmin\.cargo",
            r"C:\Users\Default\x",
            r"C:\Users\%USERNAME%\x",
            r"C:\Users\<name>",
        ]
        .join(" ");
        assert_eq!(s.scan(file.as_bytes()), []);
    }

    #[test]
    fn emails_and_long_keys_are_found() {
        let s = scanner(&[], None);
        let long = format!("sk-ant-api03-{}", made_up(1, 60, BASE62));
        let router = format!("sk-or-v1-{}", made_up(2, 64, HEX));
        let (long, router) = (format!("{long}; "), format!("{router}; "));
        let parts = ["mail ", "jane.doe@example.org; ", &long, &router];
        let at = starts(&parts);
        let hits = s.scan(parts.concat().as_bytes());
        assert_eq!(offsets(&hits, Kind::Email), [at[1]]);
        assert_eq!(offsets(&hits, Kind::KeyLike), [at[2], at[3]]);
        // Their random-looking tails are found by the text check as well.
        assert_eq!(kinds(&hits).iter().filter(|k| **k == Kind::RandomString).count(), 2);
    }

    /// The folder lines of an exclude file name the private folders; the scan finds them in any case, as text and as
    /// UTF-16.
    #[test]
    fn the_private_folders_of_the_exclude_file_are_found() {
        let exclude = "# git ls-files --others --exclude-from=.git/info/exclude\n# *.[oa]\n\n.scratch-notes/\n\
                       /drafts/\n*.log\nbuild-*/\n!keep/\n  .local-cache/  \nlocal.txt\n";
        let folders = private_folders(exclude);
        assert_eq!(folders, [".scratch-notes", "drafts", ".local-cache"]);
        let mut private = Private::default();
        for (i, folder) in folders.iter().enumerate() {
            private.add(Kind::PrivateFolder(i), folder, MIN_NAME_CHARS);
        }
        let s = Scanner::new(&private).unwrap();
        let mut file = b"see .Scratch-Notes/list.md and ".to_vec();
        let wide_at = file.len();
        file.extend(utf16le("x\\.local-cache"));
        let hits = s.scan(&file);
        assert_eq!(offsets(&hits, Kind::PrivateFolder(0)), [4]);
        assert_eq!(offsets(&hits, Kind::PrivateFolder(2)), [wide_at + 4]);
        assert_eq!(Kind::PrivateFolder(1).to_string(), "private folder 2 of .git/info/exclude");
    }

    #[test]
    fn test_keys_scoped_packages_and_css_rules_are_not_findings() {
        let s = scanner(&[], None);
        let text = r#"key "sk-ant-api03-abcdefghijklmnop-f3a9" from "@tauri-apps/api"; @font-face{} @media x"#;
        assert_eq!(s.scan(text.as_bytes()), []);
    }

    #[test]
    fn private_texts_are_found_as_utf8_and_utf16_in_any_case() {
        let key = made_up(3, 40, BASE62);
        let s = scanner(&[(Kind::SavedKey("soniox"), &key), (Kind::MicrophoneName, "Jane's AirPods")], None);
        let mut file = vec![0u8, 1, 2];
        file.extend(key.to_ascii_uppercase().as_bytes());
        let wide_at = file.len();
        file.extend(utf16le(&key));
        file.extend(b"\0headset JANE'S AIRPODS");
        let hits = s.scan(&file);
        assert_eq!(offsets(&hits, Kind::SavedKey("soniox")), [3, wide_at]);
        assert_eq!(offsets(&hits, Kind::MicrophoneName).len(), 1);
    }

    #[test]
    fn short_texts_are_not_looked_for() {
        let mut private = Private::default();
        private.add(Kind::HistoryText, "  Thank you  ", MIN_HISTORY_CHARS);
        private.add(Kind::HistoryText, "Meet Jane at the station", MIN_HISTORY_CHARS);
        private.add(Kind::DictionaryTerm(0), "AI", MIN_NAME_CHARS);
        private.add(Kind::DictionaryTerm(1), "Kubernetes", MIN_NAME_CHARS);
        private.add(Kind::MicrophoneName, "   ", MIN_NAME_CHARS);
        let looked_for: Vec<&str> = private.exact.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(looked_for, ["Meet Jane at the station", "Kubernetes"]);
    }

    #[test]
    fn the_public_name_passes_in_words_but_not_in_a_path_or_an_address() {
        let s = scanner(&[], Some(PUBLIC));
        assert_eq!(s.scan(b"CUE\nCopyright 2026 pubname\n"), []);
        assert_eq!(s.scan(&utf16le("Copyright 2026 pubname")), []);
        let parts = [r"DESKTOP-1\", "pubname | ", "pubname@desktop | ~/", "PUBNAME"];
        let at = starts(&parts);
        assert_eq!(offsets(&s.scan(parts.concat().as_bytes()), Kind::UserName), [at[1], at[2], at[3]]);
        let wide = utf16le(r"x\pubname");
        assert_eq!(offsets(&s.scan(&wide), Kind::UserName), [4]);
        // The installer's registry key under its publisher's name is no path: the name, then `\CUE` or the key's end.
        assert_eq!(s.scan(br"WriteRegStr HKCU Software\PubName\CUE"), []);
        assert_eq!(s.scan(&utf16le(r"Software\pubname\CUE")), []);
        assert_eq!(s.scan(br#"DeleteRegKey /ifempty HKCU "Software\pubname" ; and "Software\pubname\CUE\Sub""#), []);
        assert_eq!(s.scan(b"Software\\pubname\0Software\\pubname\\CUE\0"), []);
        let mut wide = utf16le(r"Software\pubname");
        wide.extend([0, 0]);
        wide.extend(utf16le(r"Software\pubname\cue"));
        assert_eq!(s.scan(&wide), []);
        // A folder of that name is a path, also under a folder called Software, in either form.
        let parts = [r"D:\Software\", r"pubname\notes | Software\", r"pubname\CUEfiles | Software\", "pubname/x"];
        let at = starts(&parts);
        let expected = [at[1], at[2], at[3]];
        assert_eq!(offsets(&s.scan(parts.concat().as_bytes()), Kind::UserName), expected);
        assert_eq!(offsets(&s.scan(&utf16le(r"D:\Software\pubname\notes")), Kind::UserName), [24]);
    }

    /// The computer name follows the user name's rule: the public name in a path (a UNC one) or an address, another
    /// name anywhere.
    #[test]
    fn the_computer_name_is_found_as_the_user_name_is() {
        let s = scanner_with_computer(&[], Some("jdoe"), Some(PUBLIC));
        assert_eq!(s.scan(b"Copyright 2026 pubname"), []);
        assert_eq!(offsets(&s.scan(br"\\pubname\share"), Kind::ComputerName), [2]);
        let s = scanner_with_computer(&[], None, Some("DESKTOP-7QK2"));
        assert_eq!(offsets(&s.scan(b"built on desktop-7qk2."), Kind::ComputerName), [9]);
        assert_eq!(offsets(&s.scan(&utf16le("on DESKTOP-7QK2")), Kind::ComputerName), [6]);
    }

    #[test]
    fn another_user_name_is_found_anywhere() {
        let s = scanner(&[], Some("jdoe"));
        assert_eq!(offsets(&s.scan(b"built by JDoe."), Kind::UserName), [9]);
        assert_eq!(offsets(&s.scan(&utf16le("by jdoe")), Kind::UserName), [6]);
    }

    #[test]
    fn a_random_string_is_found_in_text_only() {
        let s = scanner(&[], None);
        let token = made_up(4, 32, BASE62);
        let text = format!("const k = \"{token}\";");
        assert_eq!(offsets(&s.scan(text.as_bytes()), Kind::RandomString), [11]);
        let hex = made_up(5, 48, HEX);
        assert_eq!(offsets(&s.scan(format!("id={hex}\n").as_bytes()), Kind::RandomString), [3]);
        // 32 hex digits after a slash are a hash in a URL only when their line has a URL.
        let id = made_up(14, 32, HEX);
        let text = format!("keys/{id}\nsee https://x.org/{id}\n");
        assert_eq!(offsets(&s.scan(text.as_bytes()), Kind::RandomString), [5]);
        // Machine code and compressed data look random throughout: a binary gets no such check.
        let mut binary = text.into_bytes();
        binary.extend([0x8B, 0x05, 0, 0, 0, 0, 0xE8, 0x01, 0, 0, 0, 0xC3]);
        assert_eq!(s.scan(&binary), []);
    }

    /// The bars of the random check let only a few random strings through: of 1000 made-up ones, 925 of 24 letters and
    /// digits are found, 981 of 40, and all 1000 of 32 hex digits.
    #[test]
    fn random_strings_are_found_nearly_always() {
        let found = |len: usize, alphabet: &[u8]| {
            (0..1000).filter(|&seed| random_looking(made_up(100 + seed, len, alphabet).as_bytes())).count()
        };
        let (base62_24, base62_40, hex_32) = (found(24, BASE62), found(40, BASE62), found(32, HEX));
        assert!(base62_24 >= 900, "{base62_24} of 1000");
        assert!(base62_40 >= 970, "{base62_40} of 1000");
        assert!(hex_32 >= 995, "{hex_32} of 1000");
    }

    #[test]
    fn utf16_text_with_a_byte_order_mark_gets_the_random_check() {
        let s = scanner(&[], None);
        let mut file = vec![0xFF, 0xFE];
        file.extend(utf16le(&format!("x {}", made_up(6, 30, BASE62))));
        assert_eq!(offsets(&s.scan(&file), Kind::RandomString), [6]);
    }

    /// UTF-16 text without the mark, and text with a stray NUL, are text too.
    #[test]
    fn utf16_without_a_mark_and_text_with_a_nul_get_the_random_check() {
        let s = scanner(&[], None);
        let file = utf16le(&format!("name = {}\r\n", made_up(16, 30, BASE62)));
        assert_eq!(offsets(&s.scan(&file), Kind::RandomString), [14]);
        let text = format!("a\0b = \"{}\";", made_up(17, 30, BASE62));
        assert_eq!(offsets(&s.scan(text.as_bytes()), Kind::RandomString), [7]);
        // A binary's NUL units and control characters keep it out.
        let mut binary = utf16le(&made_up(18, 30, BASE62));
        binary.extend([0x01, 0x00, 0x00, 0x00]);
        assert_eq!(s.scan(&binary), []);
        // A string's closing NUL unit is text still.
        let mut ended = utf16le(&format!("name = {}", made_up(23, 30, BASE62)));
        ended.extend([0, 0]);
        assert_eq!(offsets(&s.scan(&ended), Kind::RandomString), [14]);
        // Short text with two NULs, and text in a code page (one byte that is no UTF-8), are text too.
        let text = format!("a\0b\0 = \"{}\";", made_up(24, 30, BASE62));
        assert_eq!(offsets(&s.scan(text.as_bytes()), Kind::RandomString), [8]);
        let mut code_page = b"caf\xE9 = \"".to_vec();
        code_page.extend(made_up(25, 30, BASE62).as_bytes());
        code_page.extend(b"\";\r\n");
        assert_eq!(offsets(&s.scan(&code_page), Kind::RandomString), [8]);
        // Compressed data is far from printable.
        let noise: Vec<u8> = made_up(26, 4096, BASE62).bytes().map(|b| b.wrapping_mul(149)).collect();
        assert!(View::text(&noise).is_none());
    }

    /// A key split by `-` or `_` is judged whole: a UUID, a token in parts. A Windows GUID in braces is an identifier.
    #[test]
    fn a_random_string_in_parts_is_found() {
        let s = scanner(&[], None);
        let hex = made_up(19, 32, HEX);
        let uuid = format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..]);
        let text = format!("key = {uuid};\n");
        assert_eq!(offsets(&s.scan(text.as_bytes()), Kind::RandomString), [6]);
        assert_eq!(s.scan(format!("CLSID {{{}}}\n", uuid.to_ascii_uppercase()).as_bytes()), []);
        let token = format!("tok_{}_{}_{}", made_up(20, 10, BASE62), made_up(21, 10, BASE62), made_up(22, 10, BASE62));
        assert_eq!(offsets(&s.scan(format!("\"{token}\"").as_bytes()), Kind::RandomString), [1]);
        // Base64 split by `/` and `+` into parts too short alone, judged whole once they make 32 characters.
        let base64 = format!("{}/{}+{}", made_up(27, 13, BASE62), made_up(28, 7, BASE62), made_up(29, 18, BASE62));
        assert_eq!(offsets(&s.scan(format!("key: {base64}\n").as_bytes()), Kind::RandomString), [5]);
        let short = format!("{}/{}+{}", made_up(30, 9, BASE62), made_up(31, 7, BASE62), made_up(32, 9, BASE62));
        assert_eq!(s.scan(format!("x = {short};\n").as_bytes()), []);
        let paths = "import { h } from './window/history/Page2Of4+Row3/Column7';\nn = e/1e3+t/60+Math.floor(x);\n";
        assert_eq!(s.scan(paths.as_bytes()), []);
    }

    /// The header of a made-up Ogg Opus stream's first page, as CUE writes one: `OggS`, version 0, the first-page flag,
    /// one segment, then `OpusHead`.
    fn ogg_opus_head() -> Vec<u8> {
        let mut page = b"OggS\0\x02".to_vec();
        page.extend([0; 20]);
        page.push(1);
        page.push(19);
        page.extend(b"OpusHead\x01\x01\x38\x01\x80\x3e\0\0\0\0\0");
        page
    }

    /// The header of a made-up WAV, as CUE records one: 16 kHz, mono, 16-bit PCM.
    fn wav_head(size: u32) -> Vec<u8> {
        let mut head = b"RIFF".to_vec();
        head.extend(size.to_le_bytes());
        head.extend(b"WAVEfmt ");
        head.extend(16u32.to_le_bytes());
        head.extend([1, 0, 1, 0]);
        head.extend(16_000u32.to_le_bytes());
        head.extend(32_000u32.to_le_bytes());
        head.extend([2, 0, 16, 0]);
        head.extend(b"data");
        head.extend(size.saturating_sub(36).to_le_bytes());
        head
    }

    /// A recording is found as a file of its own and as a file an installer stores as it is; a program's table of the
    /// names alone, another RIFF file or an Ogg stream of another codec is none.
    #[test]
    fn recordings_are_found_in_files_and_inside_an_installer() {
        let s = scanner(&[], None);
        let mut ogg = ogg_opus_head();
        ogg.extend([0x55; 64]);
        assert_eq!(s.scan(&ogg), [Hit { offset: 0, kind: Kind::Recording("Ogg Opus") }]);
        let mut wav = wav_head(1036);
        wav.extend([0x11; 1000]);
        assert_eq!(s.scan(&wav), [Hit { offset: 0, kind: Kind::Recording("WAV") }]);
        assert_eq!(kinds(&s.scan(&wav_head(0))), [Kind::Recording("WAV")], "cut off by a crash: no size yet");

        let mut installer = b"MZ\x90\0".to_vec();
        installer.extend([0xAB; 300]);
        let ogg_at = installer.len();
        installer.extend(&ogg);
        installer.extend([0xCD; 77]);
        let wav_at = installer.len();
        installer.extend(&wav);
        let hits = s.scan(&installer);
        assert_eq!(kinds(&hits), [Kind::Recording("Ogg Opus"), Kind::Recording("WAV")]);
        assert_eq!(hits.iter().map(|h| h.offset).collect::<Vec<_>>(), [ogg_at, wav_at]);
        assert_eq!(Kind::Recording("WAV").to_string(), "a recording (WAV)");

        let mut table = b"RIFF\0\0\0\0WAVEfmt cpal: could not create".to_vec();
        table.extend(b"RIFF\0\0\0\0WEBP\xff\xff\xff\xffOggS\0\0OpusHead");
        let mut vorbis = b"OggS\0\x02".to_vec();
        vorbis.extend([0; 20]);
        vorbis.extend([1, 30]);
        vorbis.extend(b"\x01vorbis");
        table.extend(&vorbis);
        let mut odd = wav_head(1036);
        odd[20] = 0x55;
        table.extend(&odd);
        assert_eq!(s.scan(&table), []);
    }

    /// What CUE itself writes is found: its WAV and an Ogg Opus recording of it, of a made-up tone.
    #[test]
    fn recordings_cue_writes_are_found() {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-leak-audio", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (wav, ogg) = (dir.join("tone.wav"), dir.join("tone.ogg"));
        let mut sink = cue_platform_win::wav::WavSink::create(&wav).unwrap();
        let tone: Vec<i16> = (0..3200).map(|i| ((i as f32 * 0.1).sin() * 6_000.0) as i16).collect();
        sink.write(&tone);
        sink.finish().unwrap();
        cue_platform_win::opus::encode_wav_to_ogg_opus(&wav, &ogg).unwrap();
        let (wav, ogg) = (std::fs::read(&wav).unwrap(), std::fs::read(&ogg).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        let s = scanner(&[], None);
        assert_eq!(s.scan(&wav), [Hit { offset: 0, kind: Kind::Recording("WAV") }]);
        assert_eq!(s.scan(&ogg), [Hit { offset: 0, kind: Kind::Recording("Ogg Opus") }]);
    }

    #[test]
    fn hashes_of_known_kinds_and_words_are_not_random() {
        let s = scanner(&[], None);
        let base64 = format!("{}/{}+{}==", made_up(7, 30, BASE62), made_up(8, 30, BASE62), made_up(9, 20, BASE62));
        let integrity = format!("\"integrity\": \"sha512-{base64}\"");
        let checksum = format!("checksum = \"{}\"", made_up(10, 64, HEX));
        let data = format!("url(data:font/woff2;base64,{}/{})", made_up(11, 80, BASE62), made_up(12, 40, BASE62));
        let commit = format!("https://github.com/kichik/nsis/blob/{}/Docs/attributes.but", made_up(13, 40, HEX));
        let gist = format!("; from https://gist.github.com/nikku/{}", made_up(15, 32, HEX));
        let text = [
            integrity.as_str(),
            &checksum,
            &data,
            &commit,
            &gist,
            "!define WEBVIEW2APPGUID \"{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}\"",
            "assets/Montserrat-ExtraBold-cyrillic-ext-DxGhd2a1.woff2",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
            "aVeryLongIdentifierWithoutAnyDigitsInItAtAll",
            r#"Delete "$TEMP\MicrosoftEdgeWebview2Setup.exe" "$TEMP\MicrosoftEdgeWebView2RuntimeInstaller.exe""#,
        ]
        .join("\n");
        assert_eq!(s.scan(text.as_bytes()), []);
    }

    #[test]
    fn history_texts_of_the_last_31_days_and_their_lines() {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-leak-history", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let entry = |date: &str, text: &str, question: Option<&str>| history::Entry {
            id: format!("{date}T10:00:00.000-1"),
            ts: format!("{date}T10:00:00.000"),
            mode: if question.is_some() { history::Mode::Ask } else { history::Mode::Dictation },
            app: "Notepad".into(),
            text: text.into(),
            question: question.map(str::to_owned),
            raw: None,
        };
        let heard =
            |date: &str, text: &str, raw: &str| history::Entry { raw: Some(raw.into()), ..entry(date, text, None) };
        for e in [
            entry("2026-10-31", "first line\nsecond line", None),
            heard("2026-10-30", "Cleaned up.", "uh as it was\nheard"),
            entry("2026-09-29", "32 days back: one day more for the time zone", None),
            entry("2026-10-15", "the answer", Some("the question")),
            heard("2026-09-28", "too old to look for", "too old as heard"),
        ] {
            history::append(&dir, &e).unwrap();
        }
        let texts = history_texts(&dir, "2026-10-31").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            texts,
            [
                "first line",
                "second line",
                "first line\nsecond line",
                "Cleaned up.",
                "uh as it was",
                "heard",
                "uh as it was\nheard",
                "the answer",
                "the question",
                "32 days back: one day more for the time zone"
            ]
        );
        assert_eq!(history_texts(Path::new("no such folder"), "2026-10-31").unwrap(), Vec::<String>::new());
    }

    /// A history folder that cannot be listed is no empty history: the scan stops.
    #[test]
    fn a_history_that_cannot_be_listed_stops_the_scan() {
        let file = std::env::temp_dir().join(format!("cue-test-{}-leak-history-file", std::process::id()));
        std::fs::write(&file, "not a folder").unwrap();
        let read = history_texts(&file, "2026-10-31");
        std::fs::remove_file(&file).unwrap();
        assert_eq!(read.unwrap_err().to_string(), "could not list the history folder");
    }

    #[test]
    fn what_is_printed_holds_no_private_text() {
        let mut private = Private::default();
        private.add(Kind::SavedKey("anthropic"), "made-up-secret-value", 1);
        private.add(Kind::HistoryText, "a made-up dictation text", MIN_HISTORY_CHARS);
        private.add(Kind::PrivateFolder(0), ".made-up-notes", MIN_NAME_CHARS);
        private.user_name = Some("jdoe".into());
        private.computer_name = Some("DESKTOP-7QK2".into());
        let shown = format!("{private:?} {}", private.summary());
        assert!(!shown.contains("made-up"), "{shown}");
        assert!(!shown.contains("jdoe") && !shown.to_lowercase().contains("desktop"), "{shown}");
        assert!(shown.contains("saved keys 1, history texts 1"), "{shown}");
        assert!(shown.contains("computer name yes") && shown.contains("private folders 1"), "{shown}");
        assert_eq!(Kind::ComputerName.to_string(), "the computer name");
        assert_eq!(Kind::SavedKey("soniox").to_string(), "the saved soniox key");
        assert_eq!(Kind::DictionaryTerm(2).to_string(), "dictionary term 3");
    }

    #[test]
    fn folders_are_walked_in_name_order() {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-leak-walk", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("b")).unwrap();
        std::fs::write(dir.join("b").join("2.txt"), "x").unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();
        let mut found = Vec::new();
        files(&dir, &mut found).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found, [dir.join("a.txt"), dir.join("b").join("2.txt")]);
        assert!(files(Path::new("no such folder"), &mut Vec::new()).is_err());
    }

    /// A given folder with no file in it would be scanned for nothing.
    #[test]
    fn a_given_path_with_no_files_stops_the_scan() {
        let dir = std::env::temp_dir().join(format!("cue-test-{}-leak-empty", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (full, empty) = (dir.join("full"), dir.join("empty"));
        std::fs::create_dir_all(&full).unwrap();
        std::fs::create_dir_all(empty.join("inner")).unwrap();
        std::fs::write(full.join("a.txt"), "x").unwrap();
        let listed = all_files(&[full.to_str().unwrap()]).map(|all| all.len());
        let refused = all_files(&[full.to_str().unwrap(), empty.to_str().unwrap()]).map(|all| all.len());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(listed.unwrap(), 1);
        assert!(refused.unwrap_err().to_string().starts_with("no files under "));
    }
}

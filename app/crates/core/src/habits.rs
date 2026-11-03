//! Habits (docs/specs/2026-11-04-habits.md): the same correction made again and again after a dictation becomes a rule
//! the cleanup follows. Plain code over strings: which kinds of correction a pasted text and its corrected span hold
//! (`correction`), how they are counted and learned (`Habits`), the rule's request and its template, and whether a
//! cleaned text shows a habit's effect (`marks`). The runtime watches the field and asks the cleanup's connection.
//!
//! A form is counted by its hash under this installation's secret, never by its text. A habit keeps its example, at
//! most 40 characters on each side: the only text of a correction ever written to disk. `Debug` everywhere shows
//! lengths.

use std::ops::Range;

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};

use crate::corrections;
use crate::history::{self, HabitMark};
use crate::refine;

/// The longest side of an example, in characters.
pub const MAX_EXAMPLE_CHARS: usize = 40;
/// Words of context kept around the corrected ones on each side of an example.
const CONTEXT_WORDS: usize = 2;
/// The sentence marks an end mark is made of.
const END_MARKS: [char; 4] = ['.', '!', '?', '…'];

/// The length of the hashes' secret, in bytes.
pub const SECRET_BYTES: usize = 32;

/// This installation's secret the forms' hashes are made with (docs/specs/2026-11-04-habits.md, "The hash's secret"):
/// 32 random bytes kept in Windows Credential Manager, never in settings.json, so that hashing common words does not
/// tell which ones `habits.seen` and `habits.declined` count. All zero is no secret, and nothing is counted with it.
/// `Debug` shows nothing of it; only the key store gets its hex digits (`to_hex`).
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Secret([u8; SECRET_BYTES]);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_set() { "Secret(..)" } else { "Secret(none)" })
    }
}

impl Secret {
    pub fn from_bytes(bytes: [u8; SECRET_BYTES]) -> Self {
        Self(bytes)
    }

    /// Made: not all zero.
    pub fn is_set(&self) -> bool {
        self.0.iter().any(|b| *b != 0)
    }

    /// As the key store keeps it: 64 lower-case hex digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// From the key store's 64 hex digits; anything else, or all zero, is none.
    pub fn from_hex(hex: &str) -> Option<Self> {
        if hex.len() != 2 * SECRET_BYTES || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut bytes = [0u8; SECRET_BYTES];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(Self(bytes)).filter(Secret::is_set)
    }
}

/// HMAC-SHA-256 (RFC 2104) of `message` under `key`.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| k.map(|b| b ^ byte);
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(message).finalize();
    Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize().into()
}

/// A kind of correction with its detail; counted by its [`Form::hash`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Form {
    /// The last sentence mark removed (`removed`) or added.
    EndMark { mark: char, removed: bool },
    /// The first letter's case changed: to lower case (`lower`), or to a capital.
    FirstLetter { lower: bool },
    /// A run of one to three words replaced by another, both in lower case without outer punctuation: words of the
    /// habit's example.
    Replacement { from: String, to: String },
    /// Number words replaced by digits.
    Digits,
    /// A hyphen between words replaced by a dash.
    Dash,
}

/// A replacement's words may be the user's: only their lengths.
impl std::fmt::Debug for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EndMark { mark, removed } => f.debug_struct("EndMark").field("mark", mark).field("removed", removed).finish(),
            Self::FirstLetter { lower } => f.debug_struct("FirstLetter").field("lower", lower).finish(),
            Self::Replacement { from, to } => f
                .debug_struct("Replacement")
                .field("from_chars", &from.chars().count())
                .field("to_chars", &to.chars().count())
                .finish(),
            Self::Digits => f.write_str("Digits"),
            Self::Dash => f.write_str("Dash"),
        }
    }
}

/// FNV-1a, 64 bits: a hash that never changes between versions (std's `DefaultHasher` may).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

impl Form {
    /// The kind's name, as the log and the rule request say it.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::EndMark { .. } => "endMark",
            Self::FirstLetter { .. } => "firstLetter",
            Self::Replacement { .. } => "replacement",
            Self::Digits => "digits",
            Self::Dash => "dash",
        }
    }

    /// The order a habit lists its forms in, and its rule reads them: first letter, end mark, then the rest.
    fn rank(&self) -> u8 {
        match self {
            Self::FirstLetter { .. } => 0,
            Self::EndMark { .. } => 1,
            Self::Replacement { .. } => 2,
            Self::Digits => 3,
            Self::Dash => 4,
        }
    }

    /// What the form is counted by: its kind and detail hashed under this installation's secret (HMAC-SHA-256, its
    /// first 8 bytes as 16 hex digits). Never its text, and without the secret no guess at its words.
    pub fn hash(&self, secret: &Secret) -> String {
        let canon = match self {
            Self::EndMark { mark, removed } => format!("endMark:{}{mark}", if *removed { '-' } else { '+' }),
            Self::FirstLetter { lower } => format!("firstLetter:{}", if *lower { "lower" } else { "upper" }),
            Self::Replacement { from, to } => format!("replacement:{from}\u{1f}{to}"),
            Self::Digits => "digits".to_owned(),
            Self::Dash => "dash".to_owned(),
        };
        hmac_sha256(&secret.0, canon.as_bytes())[..8].iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The form that goes back on this one (docs/specs/2026-11-04-habits.md, "Opposite forms"): the end mark added for
    /// one removed and back, the other case, the replacement the other way. Digits and a dash have none.
    pub fn opposite(&self) -> Option<Form> {
        match self {
            Self::EndMark { mark, removed } => Some(Self::EndMark { mark: *mark, removed: !removed }),
            Self::FirstLetter { lower } => Some(Self::FirstLetter { lower: !lower }),
            Self::Replacement { from, to } => Some(Self::Replacement { from: to.clone(), to: from.clone() }),
            Self::Digits | Self::Dash => None,
        }
    }

    /// Whether `text`, as the cleanup wrote it, shows this form: the end mark absent (or there), the first letter in
    /// its case, the replacement's new words there and its old ones not, digits, a dash and no spaced hyphen.
    pub fn shows_in(&self, text: &str) -> bool {
        let t = text.trim();
        match self {
            Self::EndMark { mark, removed } => t.ends_with(*mark) != *removed,
            Self::FirstLetter { lower } => t.chars().find(|c| c.is_alphabetic()).is_some_and(|c| c.is_lowercase() == *lower),
            Self::Replacement { from, to } => {
                let keys: Vec<String> = t.split_whitespace().map(corrections::key).collect();
                has_run(&keys, to) && !has_run(&keys, from)
            }
            Self::Digits => t.chars().any(|c| c.is_ascii_digit()),
            Self::Dash => (t.contains(" — ") || t.contains(" – ")) && !t.contains(" - "),
        }
    }
}

/// `words` (compared as the diff compares them) stand in a row in `keys`.
fn has_run(keys: &[String], words: &str) -> bool {
    let run: Vec<String> = words.split_whitespace().map(corrections::key).collect();
    !run.is_empty() && keys.windows(run.len()).any(|w| w == run.as_slice())
}

/// A form found in one correction, and where: its words in the pasted text (`old`) and in the corrected one (`new`).
#[derive(Clone, PartialEq, Eq)]
pub struct Found {
    pub form: Form,
    old: Range<usize>,
    new: Range<usize>,
}

impl std::fmt::Debug for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Found").field("form", &self.form).field("old", &self.old).field("new", &self.new).finish()
    }
}

/// A pasted text and its corrected span, as words, with the forms they hold. It lives in memory only.
pub struct Correction {
    old: Vec<String>,
    new: Vec<String>,
    pub found: Vec<Found>,
}

impl std::fmt::Debug for Correction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Correction")
            .field("old_words", &self.old.len())
            .field("new_words", &self.new.len())
            .field("found", &self.found)
            .finish()
    }
}

/// The kinds of correction between what CUE pasted and the corrected span, each form once, in the order first letter,
/// end mark, then the replaced runs as they stand. A text that shares no word with the paste (rewritten, another
/// message) holds none; names and terms are the dictionary's (`corrections::candidates`), never a replacement; a change
/// of case or punctuation inside the text is none of these.
pub fn correction(pasted: &str, corrected: &str) -> Correction {
    let (a, b) = (corrections::words(pasted), corrections::words(corrected));
    let mut found = Vec::new();
    let kb: Vec<String> = b.iter().map(|w| corrections::key(w)).collect();
    let shared = a.iter().map(|w| corrections::key(w)).any(|k| !k.is_empty() && kb.contains(&k));
    if shared && a.len() <= corrections::MAX_TOKENS && b.len() <= corrections::MAX_TOKENS {
        first_letter(&a, &b, &mut found);
        end_mark(&a, &b, &mut found);
        for run in corrections::runs(&a, &b) {
            replaced(&a, &b, run, corrected, &mut found);
        }
    }
    let mut forms = Vec::new();
    found.retain(|f| {
        let first = !forms.contains(&f.form);
        forms.push(f.form.clone());
        first
    });
    let own = |w: &[&str]| w.iter().map(|s| (*s).to_owned()).collect();
    Correction { old: own(&a), new: own(&b), found }
}

/// The same first word, its first letter in another case.
fn first_letter(a: &[&str], b: &[&str], out: &mut Vec<Found>) {
    let (Some(&x), Some(&y)) = (a.first(), b.first()) else { return };
    let letter = |w: &str| w.chars().find(|c| c.is_alphabetic());
    if corrections::key(x) != corrections::key(y) {
        return;
    }
    if let (Some(p), Some(q)) = (letter(x), letter(y))
        && p != q
        && p.to_lowercase().eq(q.to_lowercase())
    {
        out.push(Found { form: Form::FirstLetter { lower: q.is_lowercase() }, old: 0..1, new: 0..1 });
    }
}

/// The same last word, its sentence mark removed or added (one mark for another is neither).
fn end_mark(a: &[&str], b: &[&str], out: &mut Vec<Found>) {
    let (Some(&x), Some(&y)) = (a.last(), b.last()) else { return };
    if corrections::key(x) != corrections::key(y) {
        return;
    }
    let mark = |w: &str| w.chars().last().filter(|c| END_MARKS.contains(c));
    let form = match (mark(x), mark(y)) {
        (Some(m), None) => Form::EndMark { mark: m, removed: true },
        (None, Some(m)) => Form::EndMark { mark: m, removed: false },
        _ => return,
    };
    let (i, j) = (a.len() - 1, b.len() - 1);
    out.push(Found { form, old: i..i + 1, new: j..j + 1 });
}

/// A replaced run: a dash for a spaced hyphen, digits for number words, or ordinary words (one to three on each side,
/// each side at most `MAX_EXAMPLE_CHARS`, not a name or a term).
fn replaced(a: &[&str], b: &[&str], run: corrections::Run, corrected: &str, out: &mut Vec<Found>) {
    let (old, new) = (&a[run.old.clone()], &b[run.new.clone()]);
    let (from, to) = (old.join(" "), new.join(" "));
    let digit = |s: &str| s.chars().any(|c| c.is_ascii_digit());
    let form = if old == ["-"] && matches!(new, ["—"] | ["–"]) {
        Form::Dash
    } else if digit(&to)
        && !digit(&from)
        && from.chars().any(char::is_alphabetic)
        && !to.chars().any(char::is_uppercase)
        && !corrections::other_alphabet(&to, corrected)
    {
        Form::Digits
    } else {
        let name = corrections::looks_like_name(
            corrections::clean(&to),
            corrections::sentence_start(b, run.new.start),
            corrected,
        );
        let (from, to) = (corrections::clean(&from).to_lowercase(), corrections::clean(&to).to_lowercase());
        let fits = |s: &str| s.chars().count() <= MAX_EXAMPLE_CHARS && refine::is_term(s);
        if name || old.len() > corrections::MAX_RUN || new.len() > corrections::MAX_RUN || !fits(&from) || !fits(&to) || from == to {
            return;
        }
        Form::Replacement { from, to }
    };
    out.push(Found { form, old: run.old, new: run.new });
}

/// `s` cut to `max` characters with "…", keeping its end (`tail`) or its start.
fn cut(s: &str, max: usize, tail: bool) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_owned();
    }
    if tail {
        format!("…{}", s.chars().skip(n - (max - 1)).collect::<String>())
    } else {
        format!("{}…", s.chars().take(max - 1).collect::<String>())
    }
}

impl Correction {
    /// The example of these forms: their words with up to two words around them on each side, at most
    /// `MAX_EXAMPLE_CHARS` on each side, fewer words around when it is longer. When even their own words do not fit,
    /// a replacement's own words (they must stay in the example), else the last form's; a side still too long is cut,
    /// keeping the text's end for an end mark and its start otherwise.
    fn example(&self, found: &[&Found]) -> (String, String) {
        let span = |f: &[&Found]| {
            let lo = |r: fn(&Found) -> usize| f.iter().map(|x| r(x)).min().unwrap_or(0);
            let hi = |r: fn(&Found) -> usize| f.iter().map(|x| r(x)).max().unwrap_or(0);
            (lo(|x| x.old.start), hi(|x| x.old.end), lo(|x| x.new.start), hi(|x| x.new.end))
        };
        let side = |w: &[String], lo: usize, hi: usize, ctx: usize| w[lo.saturating_sub(ctx)..(hi + ctx).min(w.len())].join(" ");
        let fit = |f: &[&Found]| {
            let (a0, a1, b0, b1) = span(f);
            (0..=CONTEXT_WORDS)
                .rev()
                .map(|c| (side(&self.old, a0, a1, c), side(&self.new, b0, b1, c)))
                .find(|(x, y)| x.chars().count() <= MAX_EXAMPLE_CHARS && y.chars().count() <= MAX_EXAMPLE_CHARS)
        };
        if let Some(e) = fit(found) {
            return e;
        }
        let replacement: Vec<&Found> =
            found.iter().copied().filter(|f| matches!(f.form, Form::Replacement { .. })).take(1).collect();
        if !replacement.is_empty()
            && let Some(e) = fit(&replacement)
        {
            return e;
        }
        let last = &found[found.len() - 1..];
        let (a0, a1, b0, b1) = span(last);
        let tail = matches!(last[0].form, Form::EndMark { .. });
        let side0 = |w: &[String], lo, hi| cut(&side(w, lo, hi, 0), MAX_EXAMPLE_CHARS, tail);
        (side0(&self.old, a0, a1), side0(&self.new, b0, b1))
    }
}

/// A form becomes a habit when it was corrected this many times in one app.
pub const LEARN_AT: u32 = 3;
/// A count not seen for this many days is dropped.
pub const SEEN_DAYS: u32 = 30;
/// The most counts kept; the least recently seen go first.
pub const MAX_SEEN: usize = 300;
/// The most habits kept; the oldest go first.
pub const MAX_HABITS: usize = 50;
/// The longest rule, and short name, a habit takes.
pub const MAX_RULE_CHARS: usize = 80;
pub const MAX_SHORT_CHARS: usize = 32;
/// How long the rule request may take before the template writes the rule.
pub const RULE_WAIT_MS: u64 = 5_000;

/// How many times a form was corrected in an app, or a habit of another app was corrected there whole, and the day it
/// last was: no text (`form` is the form's hash, or the habit's id).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Seen {
    pub app: String,
    pub form: String,
    pub n: u32,
    pub last: String,
}

/// The app's name is the user's: only its length.
impl std::fmt::Debug for Seen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Seen")
            .field("app_chars", &self.app.chars().count())
            .field("form", &self.form)
            .field("n", &self.n)
            .field("last", &self.last)
            .finish()
    }
}

fn yes() -> bool {
    true
}

/// A habit: its forms (each kind with its detail), the rule the cleanup follows and its short name, the example, the
/// app it holds in (`None`: everywhere), how many times it was corrected, and whether it is on.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Habit {
    pub id: String,
    pub forms: Vec<Form>,
    pub rule: String,
    pub short: String,
    pub before: String,
    pub after: String,
    #[serde(default)]
    pub app: Option<String>,
    pub n: u32,
    #[serde(default = "yes")]
    pub on: bool,
}

impl std::fmt::Debug for Habit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Habit")
            .field("id", &self.id)
            .field("forms", &self.forms)
            .field("rule_chars", &self.rule.chars().count())
            .field("short_chars", &self.short.chars().count())
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .field("everywhere", &self.app.is_none())
            .field("n", &self.n)
            .field("on", &self.on)
            .finish()
    }
}

impl Habit {
    pub fn hashes(&self, secret: &Secret) -> Vec<String> {
        self.forms.iter().map(|f| f.hash(secret)).collect()
    }

    /// Its effect shows in `text`: every one of its forms does.
    pub fn applied(&self, text: &str) -> bool {
        !self.forms.is_empty() && self.forms.iter().all(|f| f.shows_in(text))
    }
}

/// A list of items as settings.json holds them: anything but a list reads as none, and an item that does not read (a
/// later version's, a typo) is skipped.
fn items_or_nothing<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    let serde_json::Value::Array(items) = value else { return Ok(Vec::new()) };
    Ok(items.into_iter().filter_map(|item| serde_json::from_value(item).ok()).collect())
}

/// `habits` in settings.json: the counts (`seen`, hashes only), the habits (`list`, the newest first) and the forms
/// never to learn again (`declined`, hashes only). Each list is written only when it is not empty. The secret the hashes
/// are made with is never written: the runtime sets it from the key store.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Habits {
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub seen: Vec<Seen>,
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub list: Vec<Habit>,
    #[serde(deserialize_with = "items_or_nothing", skip_serializing_if = "Vec::is_empty")]
    pub declined: Vec<String>,
    #[serde(skip)]
    pub secret: Secret,
}

/// A habit being made: its forms, the app and the example, and how many times they were corrected. Its rule comes
/// from the cleanup's connection or the template ([`Habits::adopt`]).
#[derive(Clone, PartialEq)]
pub struct Draft {
    pub forms: Vec<Form>,
    pub app: String,
    pub before: String,
    pub after: String,
    pub n: u32,
}

impl std::fmt::Debug for Draft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Draft")
            .field("forms", &self.forms)
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .field("n", &self.n)
            .finish()
    }
}

/// What the rule request carries, and nothing else: the kinds, the example and the app's name.
#[derive(Clone, PartialEq, Serialize)]
pub struct RuleAsk {
    pub kind: Vec<&'static str>,
    pub before: String,
    pub after: String,
    pub app: String,
}

impl std::fmt::Debug for RuleAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleAsk")
            .field("kind", &self.kind)
            .field("before_chars", &self.before.chars().count())
            .field("after_chars", &self.after.chars().count())
            .finish()
    }
}

impl Draft {
    pub fn ask(&self) -> RuleAsk {
        RuleAsk { kind: self.forms.iter().map(Form::kind).collect(), before: self.before.clone(), after: self.after.clone(), app: self.app.clone() }
    }
}

/// A habit's rule, a sentence of at most `MAX_RULE_CHARS`, and its short name, one to three words.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct RuleText {
    pub rule: String,
    pub short: String,
}

impl std::fmt::Debug for RuleText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleText")
            .field("rule_chars", &self.rule.chars().count())
            .field("short_chars", &self.short.chars().count())
            .finish()
    }
}

/// Forms that go back on each other: one of `a` has its opposite in `b`.
fn opposes(a: &[Form], b: &[Form]) -> bool {
    a.iter().any(|f| f.opposite().is_some_and(|o| b.contains(&o)))
}

/// Two habits that may not live together: they go back on each other in one app, or one of them holds everywhere.
fn clash(a: &Habit, b: &Habit) -> bool {
    opposes(&a.forms, &b.forms) && (a.app.is_none() || b.app.is_none() || a.app == b.app)
}

/// What learning a draft did ([`Habits::learn`]).
#[derive(Debug, PartialEq)]
pub struct Learned {
    /// The habit as it is now.
    pub habit: Habit,
    /// The habits it changed, as they were: they held an opposite of its forms and are gone, not declined.
    pub replaced: Vec<Habit>,
    /// A habit of its forms was listed already in its app or everywhere (a draft of the same forms came first, or the
    /// habit widened while the draft waited): the draft added to it.
    pub merged: bool,
    /// That habit as it was before the draft added to it (`merged`), to take the change back without a trace.
    pub was: Option<Habit>,
}

/// What counting one correction changed.
#[derive(Debug, Default, PartialEq)]
pub struct Counted {
    /// Forms that reached `LEARN_AT` in this app together: the habit to make.
    pub new: Option<Draft>,
    /// Habits of another app whose forms came together here `LEARN_AT` times: they hold everywhere now. Each id a
    /// habit had, with the id of everywhere it has now (habits of the same forms in other apps fold into it).
    pub widened: Vec<(String, String)>,
    /// A habit's form was corrected again: its count grew.
    pub grown: bool,
}

impl Habits {
    /// Nothing counted, learned or declined (the secret aside).
    pub fn is_default(&self) -> bool {
        self.seen.is_empty() && self.list.is_empty() && self.declined.is_empty()
    }

    /// The habits that go into a dictation's cleanup request in `app`: on, and for that app or everywhere (an unknown
    /// app, `""`, gets the ones of everywhere).
    pub fn for_app<'a>(&'a self, app: &'a str) -> impl Iterator<Item = &'a Habit> + 'a {
        self.list.iter().filter(move |h| h.on && h.app.as_deref().is_none_or(|a| !app.is_empty() && a == app))
    }

    /// One correction of a dictation in `app` (the history's name for it, `today` its local date): each form counts
    /// once. A declined form is not counted. A form of a habit of this app or of everywhere adds to that habit's count.
    /// A habit of another app whose forms all came here together counts as one entry of this app, under its id; at
    /// `LEARN_AT` it widens (its rule loses its app, it takes the id of everywhere, its counts add up, and habits of
    /// the same forms in other apps fold into it). A habit with an opposite habit in some app (`opposed`) has no set:
    /// it never widens. The other forms count for this app, an opposite of a habit's
    /// form among them, and those that reach `LEARN_AT` together become one draft. The counts that went into a habit
    /// go; counts older than `SEEN_DAYS` go, and past `MAX_SEEN` the least recently seen. Without the secret nothing is
    /// counted.
    pub fn count(&mut self, app: &str, c: &Correction, today: &str, lang: &str) -> Counted {
        let mut out = Counted::default();
        if app.is_empty() || !self.secret.is_set() {
            return out;
        }
        self.prune(today);
        let live: Vec<(&Found, String)> =
            c.found.iter().map(|f| (f, f.form.hash(&self.secret))).filter(|(_, hash)| !self.declined.contains(hash)).collect();
        // The forms counted on a habit already, or in a habit's set.
        let mut handled: Vec<String> = Vec::new();
        let mut grown: Vec<String> = Vec::new();
        for (_, hash) in &live {
            let here = |h: &&Habit| h.app.as_deref().is_none_or(|a| a == app) && h.hashes(&self.secret).contains(hash);
            if let Some(h) = self.list.iter().find(here) {
                if !grown.contains(&h.id) {
                    grown.push(h.id.clone());
                }
                handled.push(hash.clone());
            }
        }
        let free: Vec<&String> = live.iter().map(|(_, hash)| hash).filter(|hash| !handled.contains(hash)).collect();
        let sets: Vec<(String, Vec<String>)> = self
            .list
            .iter()
            .filter(|h| h.app.as_deref().is_some_and(|a| a != app) && !self.opposed(&h.forms, &h.id))
            .map(|h| (h.id.clone(), h.hashes(&self.secret)))
            .filter(|(_, hashes)| !hashes.is_empty() && hashes.iter().all(|hash| free.contains(&hash)))
            .collect();
        let mut widened: Vec<(String, u32)> = Vec::new();
        for (id, hashes) in sets {
            handled.extend(hashes);
            let n = self.bump(app, &id, today);
            if n >= LEARN_AT {
                widened.push((id, n));
            }
        }
        // One folded into a habit widened before it in this loop has gone: its set counted the same corrections.
        for (id, n) in &widened {
            if let Some(i) = self.list.iter().position(|h| &h.id == id) {
                self.list[i].n = self.list[i].n.saturating_add(*n);
                if let Some((at, had)) = self.widen(i, lang) {
                    let all = &self.list[at].id;
                    out.widened.extend(had.into_iter().map(|had| (had, all.clone())));
                }
            }
        }
        // A habit's count grows once per correction.
        for h in &mut self.list {
            if grown.contains(&h.id) {
                h.n = h.n.saturating_add(1);
            }
        }
        out.grown = !grown.is_empty();
        let mut reached: Vec<&Found> = Vec::new();
        for (f, hash) in &live {
            if !handled.contains(hash) && self.bump(app, hash, today) >= LEARN_AT {
                reached.push(f);
            }
        }
        // Only replacements whose words stand in the stored example are drafted; the others keep their counts.
        let mut example = (String::new(), String::new());
        while !reached.is_empty() {
            example = c.example(&reached);
            let (before, after) = (&example.0, &example.1);
            let keys = |t: &str| t.split_whitespace().map(corrections::key).collect::<Vec<_>>();
            let (kb, ka) = (keys(before), keys(after));
            let before_len = reached.len();
            reached.retain(|f| match &f.form {
                Form::Replacement { from, to } => has_run(&kb, from) && has_run(&ka, to),
                _ => true,
            });
            if reached.len() == before_len {
                break;
            }
        }
        if !reached.is_empty() {
            let gone: Vec<String> = reached.iter().map(|f| f.form.hash(&self.secret)).collect();
            self.seen.retain(|s| s.app != app || !gone.contains(&s.form));
            let mut forms: Vec<Form> = reached.iter().map(|f| f.form.clone()).collect();
            forms.sort_by_key(Form::rank);
            let (before, after) = example;
            out.new = Some(Draft { forms, app: app.to_owned(), before, after, n: LEARN_AT });
        }
        out
    }

    /// The form's count in `app` grows by one and goes first (the most recently seen); the new count.
    fn bump(&mut self, app: &str, form: &str, today: &str) -> u32 {
        let n = match self.seen.iter().position(|s| s.app == app && s.form == form) {
            Some(i) => self.seen.remove(i).n.saturating_add(1),
            None => 1,
        };
        self.seen.insert(0, Seen { app: app.to_owned(), form: form.to_owned(), n, last: today.to_owned() });
        self.seen.truncate(MAX_SEEN);
        n
    }

    /// Counts last seen before the `SEEN_DAYS` before `today` go (none when `today` is not a date).
    fn prune(&mut self, today: &str) {
        if let Some(first_kept) = history::date_minus(today, SEEN_DAYS) {
            self.seen.retain(|s| s.last.as_str() >= first_kept.as_str());
        }
    }

    /// The draft's habit, as [`Habits::learn`] makes it.
    pub fn adopt(&mut self, draft: Draft, rule: Option<RuleText>, lang: &str) -> Habit {
        self.learn(draft, rule, lang).habit
    }

    /// The draft's habit goes first, with the rule the cleanup wrote or else the template's; past `MAX_HABITS` the
    /// oldest go. Its id is made from its forms and its app (`id_of`). The habits holding an opposite of its forms in
    /// its app or everywhere are changed by it (docs/specs/2026-11-04-habits.md, "Changed"), switched-off ones too: they
    /// go, not declined; when one of them held everywhere, the new habit does too, and those of every app go as well.
    /// When a habit of its forms is listed already in its app or everywhere (a draft of the same forms reached three in
    /// the same app again while the first waited for its rule, or the habit widened meanwhile), the draft adds to it
    /// instead: its count adds up, and the draft's rule is not used; `was` keeps it as it was. A habit of another app is
    /// never added to: the draft makes this app's own habit.
    pub fn learn(&mut self, draft: Draft, rule: Option<RuleText>, lang: &str) -> Learned {
        let opposite = |h: &Habit| opposes(&h.forms, &draft.forms);
        let everywhere = self.list.iter().any(|h| h.app.is_none() && opposite(h));
        let (replaced, kept): (Vec<Habit>, Vec<Habit>) = std::mem::take(&mut self.list)
            .into_iter()
            .partition(|h| opposite(h) && (everywhere || h.app.as_deref() == Some(draft.app.as_str())));
        self.list = kept;
        let here = id_of(&draft.forms, Some(&draft.app), &self.secret);
        let all = id_of(&draft.forms, None, &self.secret);
        if let Some(i) = self.list.iter().position(|h| h.id == here || h.id == all) {
            let was = self.list[i].clone();
            self.list[i].n = self.list[i].n.saturating_add(draft.n);
            let at = if everywhere { self.widen(i, lang).map_or(i, |(at, _)| at) } else { i };
            return Learned { habit: self.list[at].clone(), replaced, merged: true, was: Some(was) };
        }
        let RuleText { rule, short } = rule.unwrap_or_else(|| template(&draft.forms, Some(&draft.app), lang));
        let (id, rule, app) = if everywhere {
            (all, unscoped(&rule, &draft.app, &draft.forms, lang), None)
        } else {
            (here, rule, Some(draft.app))
        };
        let habit = Habit { id, forms: draft.forms, rule, short, before: draft.before, after: draft.after, app, n: draft.n, on: true };
        self.list.insert(0, habit.clone());
        self.list.truncate(MAX_HABITS);
        Learned { habit, replaced, merged: false, was: None }
    }

    /// The habit at `i` holds everywhere from now on: its app is cleared, its rule loses it (`unscoped`), and it takes
    /// the id of everywhere. Habits of the same forms in other apps fold into it (their counts add up), and the counts
    /// of its forms and of the sets counted under any of their ids go, in every app: none follows the new id. Where it
    /// is in the list now and the ids that became its id; `None` when it held everywhere already.
    fn widen(&mut self, i: usize, lang: &str) -> Option<(usize, Vec<String>)> {
        let h = &mut self.list[i];
        let app = h.app.take()?;
        h.rule = unscoped(&h.rule, &app, &h.forms, lang);
        let forms = h.forms.clone();
        let mut had = vec![std::mem::replace(&mut h.id, id_of(&forms, None, &self.secret))];
        let (mut folded, mut before, mut at) = (0u32, 0, 0);
        self.list.retain(|x| {
            let fold = at != i && x.forms == forms;
            if fold {
                had.push(x.id.clone());
                folded = folded.saturating_add(x.n);
                before += usize::from(at < i);
            }
            at += 1;
            !fold
        });
        let i = i - before;
        self.list[i].n = self.list[i].n.saturating_add(folded);
        let mut gone: Vec<String> = forms.iter().map(|f| f.hash(&self.secret)).collect();
        gone.extend(had.iter().cloned());
        self.seen.retain(|s| !gone.contains(&s.form));
        Some((i, had))
    }

    /// A habit other than `id` holds an opposite of one of `forms`: a habit of them never widens, as it would then live
    /// together with that one.
    fn opposed(&self, forms: &[Form], id: &str) -> bool {
        self.list.iter().any(|h| h.id != id && opposes(&h.forms, forms))
    }

    /// A change undone: the habit `id` goes and the habits it replaced come back as they were, first (a habit of the
    /// same id among them takes the place of the one listed). With `decline` (the card's "Undo") its forms are never
    /// learned again; without (a change dropped before its card, "Learn from me" off) they may be. The undo wins: a
    /// habit learned or widened since that goes back on one brought back, in its app or everywhere, goes too, not
    /// declined. The habits that went so; `None` when there is no habit `id`.
    pub fn revert(&mut self, id: &str, replaced: Vec<Habit>, decline: bool) -> Option<Vec<Habit>> {
        let gone = if decline {
            self.forget(id)
        } else {
            let before = self.list.len();
            self.list.retain(|h| h.id != id);
            self.list.len() != before
        };
        if !gone {
            return None;
        }
        let mut removed = Vec::new();
        for h in replaced.into_iter().rev() {
            let (out, kept): (Vec<Habit>, Vec<Habit>) =
                std::mem::take(&mut self.list).into_iter().partition(|x| x.id == h.id || clash(x, &h));
            self.list = kept;
            removed.extend(out.into_iter().filter(|x| x.id != h.id));
            self.list.insert(0, h);
        }
        self.list.truncate(MAX_HABITS);
        removed.retain(|x| !self.list.iter().any(|h| h.id == x.id));
        Some(removed)
    }

    /// The habit goes, and its forms are declined: never learned again; its set's counts go too. Without the secret
    /// nothing is declined: a hash under no secret is easy to guess and would match nothing (nothing is counted then
    /// either). False when there is no such habit.
    pub fn forget(&mut self, id: &str) -> bool {
        let Some(i) = self.list.iter().position(|h| h.id == id) else { return false };
        self.seen.retain(|s| s.form != id);
        let habit = self.list.remove(i);
        if !self.secret.is_set() {
            return true;
        }
        for hash in habit.hashes(&self.secret) {
            if !self.declined.contains(&hash) {
                self.declined.push(hash);
            }
        }
        true
    }

    /// Every habit goes, with the counts and the declined forms; the secret stays. False when there was nothing.
    pub fn forget_all(&mut self) -> bool {
        let changed = !self.is_default();
        self.seen.clear();
        self.list.clear();
        self.declined.clear();
        changed
    }

    /// A new secret, the one kept being missing or malformed: the counts and the declined forms were hashed with the
    /// old one and go; the habits keep their forms, and their ids are made again. How many counts and declined forms
    /// went.
    pub fn rekey(&mut self, secret: Secret) -> (usize, usize) {
        let dropped = (self.seen.len(), self.declined.len());
        self.seen.clear();
        self.declined.clear();
        self.secret = secret;
        self.reid();
        dropped
    }

    /// Every habit's id made again under the secret (`id_of`), where it differs (a settings.json restored or brought
    /// from another computer): a set counted under a habit's old id follows it. True when one changed.
    pub fn reid(&mut self) -> bool {
        let mut changed = false;
        for h in &mut self.list {
            let id = id_of(&h.forms, h.app.as_deref(), &self.secret);
            if h.id != id {
                for s in self.seen.iter_mut().filter(|s| s.form == h.id) {
                    s.form.clone_from(&id);
                }
                h.id = id;
                changed = true;
            }
        }
        changed
    }

    /// A habit on or off; false when nothing changed.
    pub fn switch(&mut self, id: &str, on: bool) -> bool {
        self.list.iter_mut().find(|h| h.id == id).is_some_and(|h| std::mem::replace(&mut h.on, on) != on)
    }
}

/// A habit's id: the hash of its forms' hashes (made with the secret) and its app (`None`: everywhere), so the habits
/// of the same forms in two apps are two habits.
pub fn id_of(forms: &[Form], app: Option<&str>, secret: &Secret) -> String {
    let mut key = forms.iter().map(|f| f.hash(secret)).collect::<Vec<_>>().join(",");
    if let Some(app) = app {
        key.push('@');
        key.push_str(app);
    }
    format!("h{:016x}", fnv1a(key.as_bytes()))
}

/// `s` with its first letter a capital.
fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|first| first.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// One form's part of a template rule, and its short name.
fn part(form: &Form, ru: bool) -> (String, String) {
    let own = |rule: &str, short: &str| (rule.to_owned(), short.to_owned());
    match (form, ru) {
        (Form::EndMark { mark: '.', removed: true }, false) => own("no full stop at the end", "no full stop"),
        (Form::EndMark { mark: '.', removed: true }, true) => own("без точки в конце", "без точки"),
        (Form::EndMark { mark, removed: true }, false) => (format!("no “{mark}” at the end"), format!("no “{mark}”")),
        (Form::EndMark { mark, removed: true }, true) => (format!("без «{mark}» в конце"), format!("без «{mark}»")),
        (Form::EndMark { mark: '.', removed: false }, false) => own("a full stop at the end", "full stop"),
        (Form::EndMark { mark: '.', removed: false }, true) => own("с точкой в конце", "с точкой"),
        (Form::EndMark { mark, removed: false }, false) => (format!("“{mark}” at the end"), format!("with “{mark}”")),
        (Form::EndMark { mark, removed: false }, true) => (format!("«{mark}» в конце"), format!("с «{mark}»")),
        (Form::FirstLetter { lower: true }, false) => own("lower case at the start", "lower case"),
        (Form::FirstLetter { lower: true }, true) => own("с маленькой буквы", "с маленькой"),
        (Form::FirstLetter { lower: false }, false) => own("a capital at the start", "capital"),
        (Form::FirstLetter { lower: false }, true) => own("с большой буквы", "с большой"),
        (Form::Replacement { from, to }, false) => (format!("write “{to}” instead of “{from}”"), cut(to, MAX_SHORT_CHARS, false)),
        (Form::Replacement { from, to }, true) => (format!("писать «{to}» вместо «{from}»"), cut(to, MAX_SHORT_CHARS, false)),
        (Form::Digits, false) => own("numbers in digits", "digits"),
        (Form::Digits, true) => own("числа цифрами", "цифрами"),
        (Form::Dash, false) => own("a dash between words, not a hyphen", "dash"),
        (Form::Dash, true) => own("тире вместо дефиса", "тире"),
    }
}

/// The rule when the cleanup's connection gave none: `In <app>: …` / `В <app>: …` with each form's part (the last
/// joined by "and" / «и»), or without the app for everywhere; the short name is the last form's. At most
/// `MAX_RULE_CHARS`.
pub fn template(forms: &[Form], app: Option<&str>, lang: &str) -> RuleText {
    let ru = lang == "ru";
    let parts: Vec<(String, String)> = forms.iter().map(|f| part(f, ru)).collect();
    let and = if ru { " и " } else { " and " };
    let what = match parts.as_slice() {
        [] => String::new(),
        [one] => one.0.clone(),
        [init @ .., last] => format!("{}{and}{}", init.iter().map(|p| p.0.as_str()).collect::<Vec<_>>().join(", "), last.0),
    };
    let rule = match app {
        Some(app) => format!("{} {app}: {what}", if ru { "В" } else { "In" }),
        None => capitalized(&what),
    };
    let short = parts.last().map(|p| p.1.clone()).unwrap_or_default();
    RuleText { rule: cut(&rule, MAX_RULE_CHARS, false), short }
}

/// A rule written for one app, for every app: its `In <app>` / `В <app>` start cut off (with a dash, a colon or a
/// comma after it, so the name is whole), the rest from a capital; a rule that does not start so is written anew from the template.
pub fn unscoped(rule: &str, app: &str, forms: &[Form], lang: &str) -> String {
    for lead in ["In ", "in ", "В ", "в "] {
        if let Some(rest) = rule.strip_prefix(lead).and_then(|r| r.strip_prefix(app)) {
            let tail = rest.trim_start_matches([' ', ':', '—', '–', '-', ',']);
            // The app's name ends at a word boundary: a mark follows it ("In Telegram — …"), not more words of a longer name.
            let marked = rest[..rest.len() - tail.len()].contains([':', '—', '–', '-', ',']);
            if marked && !tail.is_empty() {
                return capitalized(tail);
            }
        }
    }
    template(forms, None, lang).rule
}

/// The rule request's instructions; `{LANGUAGE}` becomes the interface language.
const RULE_PROMPT: &str = "\
A dictation app pastes text that the user then corrects by hand, and the user has made the same correction three \
times in one app. From the kind of correction (endMark: the last sentence mark removed or added; firstLetter: the first \
letter's case changed; replacement: words written another way; digits: numbers written as digits; dash: a dash between \
words instead of a hyphen), one example (the text before and after the correction) and the app's name, write the rule \
the app should follow there from now on. \"rule\": one sentence of at most 80 characters that starts with the app, like \
\"In Telegram — lower case and no full stop\". \"short\": one to three words that name it, like \"no full stop\". No full \
stop at the end of either. Write both in {LANGUAGE}. Answer with a JSON object with exactly the fields \"rule\" and \
\"short\".";

/// The rule request's two messages: the instructions, and the user message — the kinds, the example and the app as a
/// JSON object, nothing else.
pub fn rule_messages(ask: &RuleAsk, lang: &str) -> (String, String) {
    let language = if lang == "ru" { "Russian" } else { "English" };
    (RULE_PROMPT.replace("{LANGUAGE}", language), serde_json::to_string(ask).unwrap_or_default())
}

/// One line: no control character, no line or paragraph separator.
pub(crate) fn one_line(s: &str) -> bool {
    !s.chars().any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
}

/// The cleanup's answer to the rule request: a JSON object with `rule` and `short`, in a code fence or not. Usable when
/// each is one line, the rule not empty and at most `MAX_RULE_CHARS`, the short name one to three words and at most
/// `MAX_SHORT_CHARS`; a full stop at their end goes.
pub fn read_rule(content: &str) -> Option<RuleText> {
    let t = content.trim();
    let t = match t.strip_prefix("```json").or_else(|| t.strip_prefix("```")) {
        Some(inner) => inner.trim_end().strip_suffix("```").unwrap_or(inner),
        None => t,
    };
    let r: RuleText = serde_json::from_str(t.trim()).ok()?;
    let rule = r.rule.trim().trim_end_matches('.').trim_end().to_owned();
    let short = r.short.trim().trim_end_matches('.').trim_end().to_owned();
    let words = short.split_whitespace().count();
    let ok = !rule.is_empty()
        && rule.chars().count() <= MAX_RULE_CHARS
        && one_line(&rule)
        && (1..=3).contains(&words)
        && short.chars().count() <= MAX_SHORT_CHARS
        && one_line(&short);
    ok.then_some(RuleText { rule, short })
}

/// Of the habits sent with a dictation's cleanup, the ones whose effect the cleanup brought: it shows in the cleaned
/// text and not in the text as heard (`raw`), which may have had it already (digits, no full stop). Its history entry
/// lists them.
pub fn marks(sent: &[Habit], raw: &str, text: &str) -> Vec<HabitMark> {
    sent.iter()
        .filter(|h| h.applied(text) && !h.applied(raw))
        .map(|h| HabitMark { id: h.id.clone(), short: h.short.clone() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(pasted: &str, corrected: &str) -> Vec<Form> {
        correction(pasted, corrected).found.into_iter().map(|f| f.form).collect()
    }
    fn example(pasted: &str, corrected: &str) -> (String, String) {
        let c = correction(pasted, corrected);
        let all: Vec<&Found> = c.found.iter().collect();
        c.example(&all)
    }
    const STOP: Form = Form::EndMark { mark: '.', removed: true };
    fn key(byte: u8) -> Secret {
        Secret::from_bytes([byte; SECRET_BYTES])
    }
    /// The habits with a secret, as the runtime holds them.
    fn habits() -> Habits {
        Habits { secret: key(7), ..Habits::default() }
    }

    #[test]
    fn the_end_mark_and_the_first_letter_in_one_correction() {
        assert_eq!(kinds("Скинь файл.", "скинь файл"), [Form::FirstLetter { lower: true }, STOP]);
    }

    #[test]
    fn an_end_mark_added_or_removed_but_not_swapped() {
        assert_eq!(kinds("привет мир", "привет, мир!"), [Form::EndMark { mark: '!', removed: false }]);
        assert_eq!(kinds("Посмотрю вечером…", "Посмотрю вечером"), [Form::EndMark { mark: '…', removed: true }]);
        assert!(kinds("Готово!", "Готово?").is_empty(), "one mark for another is neither");
    }

    #[test]
    fn a_capital_given_is_a_first_letter_too() {
        assert_eq!(kinds("скинь файл", "Скинь файл"), [Form::FirstLetter { lower: false }]);
    }

    #[test]
    fn a_replacement_of_ordinary_words() {
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert_eq!(kinds("Здравствуйте, Анна!", "Добрый день, Анна!"), [hello]);
    }

    #[test]
    fn names_are_left_to_the_dictionary() {
        assert!(kinds("Мне нужно позвонить Марисоль.", "Мне нужно позвонить Marisol.").is_empty());
        assert!(kinds("открой гитхаб сейчас", "открой GitHub сейчас").is_empty());
    }

    #[test]
    fn digits_for_number_words() {
        assert_eq!(kinds("рост двенадцать процентов за год", "рост 12 % за год"), [Form::Digits]);
    }

    #[test]
    fn digits_for_number_words_but_not_for_a_term() {
        assert!(kinds("спроси гпт четыре сегодня", "спроси GPT-4 сегодня").is_empty());
        assert_eq!(kinds("через пять минут", "через 5 минут"), [Form::Digits]);
    }

    #[test]
    fn a_dash_for_a_hyphen_once_per_correction() {
        assert_eq!(kinds("клиенты - в основном малый бизнес", "клиенты — в основном малый бизнес"), [Form::Dash]);
        assert_eq!(kinds("ну - да - конечно", "ну — да — конечно"), [Form::Dash], "a form counts once");
    }

    #[test]
    fn case_or_punctuation_inside_the_text_is_no_habit() {
        assert!(kinds("позвони github завтра", "позвони GitHub завтра").is_empty(), "case only");
        assert!(kinds("да нет наверное", "да, нет, наверное").is_empty(), "commas only");
    }

    #[test]
    fn a_rewritten_text_holds_nothing() {
        assert!(kinds("Скинь файл.", "Позвоню позже").is_empty());
    }

    /// HMAC-SHA-256 under the secret: the same form and secret give the same hash in every version, so a count
    /// survives an update; another secret gives another hash.
    #[test]
    fn the_hash_is_stable_under_one_secret_and_another_under_another() {
        let k = key(1);
        assert_eq!(Form::Digits.hash(&k), "0cda9c2120dc4085");
        assert_eq!(Form::Dash.hash(&k), "1e280b5650f82678");
        assert_eq!(STOP.hash(&k), "d61bb7341f7ef877");
        assert_eq!(Form::FirstLetter { lower: true }.hash(&k), "1465766befda9e4e");
        assert_eq!(Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() }.hash(&k), "06f5620c5fa0f9db");
        assert_eq!(Form::Digits.hash(&k), Form::Digits.hash(&key(1)), "the same form and secret");
        assert_ne!(Form::EndMark { mark: '.', removed: false }.hash(&k), STOP.hash(&k));
        for form in [Form::Digits, Form::Dash, STOP] {
            assert_ne!(form.hash(&k), form.hash(&key(2)), "{form:?} under another secret");
        }
        assert_eq!(Form::Digits.hash(&key(2)), "d74d04473faae824");
    }

    /// RFC 4231, test case 2.
    #[test]
    fn hmac_sha256_is_the_standard_one() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    /// RFC 4231, test case 6: a key longer than the block is hashed first.
    #[test]
    fn hmac_sha256_hashes_a_key_longer_than_the_block_first() {
        let mac = hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn the_secret_reads_back_from_its_hex_digits_and_shows_nothing() {
        let k = key(0x5c);
        assert_eq!(k.to_hex(), "5c".repeat(SECRET_BYTES));
        assert_eq!(Secret::from_hex(&k.to_hex()), Some(k.clone()));
        let (short, long, letters, signs, zero) =
            ("5c".to_owned(), "5c".repeat(SECRET_BYTES + 1), "zz".repeat(SECRET_BYTES), "+5".repeat(SECRET_BYTES), "00".repeat(SECRET_BYTES));
        for bad in ["", &short, &long, &letters, &signs, &zero] {
            assert_eq!(Secret::from_hex(bad), None, "{bad}");
        }
        assert_eq!(format!("{k:?}"), "Secret(..)");
        assert_eq!(format!("{:?}", Secret::default()), "Secret(none)");
        let h = Habits { secret: k, ..Habits::default() };
        let shown = format!("{h:?}");
        assert!(!shown.contains("5c5c") && !shown.contains("92, 92"), "{shown}");
        assert_eq!(serde_json::to_value(&h).unwrap(), serde_json::json!({}), "never written");
    }

    #[test]
    fn the_example_is_the_corrected_words_with_two_around() {
        assert_eq!(example("Скинь файл.", "скинь файл"), ("Скинь файл.".into(), "скинь файл".into()));
        assert_eq!(
            example("Мы выросли на двенадцать процентов за этот год", "Мы выросли на 12 % за этот год"),
            ("выросли на двенадцать процентов за этот".into(), "выросли на 12 % за этот".into()),
        );
    }

    #[test]
    fn an_example_side_is_never_longer_than_forty_characters() {
        let pasted = "Пожалуйста посмотрите приложенный квартальный отчёт до вечера пятницы.";
        let corrected = "Пожалуйста посмотрите приложенный квартальный отчёт до вечера пятницы";
        assert_eq!(example(pasted, corrected), ("до вечера пятницы.".into(), "до вечера пятницы".into()));
        let one = "Превысокомногорассмотрительствующий-предлинный-пример.";
        let (before, after) = example(one, one.trim_end_matches('.'));
        assert_eq!((before.chars().count(), after.chars().count()), (MAX_EXAMPLE_CHARS, MAX_EXAMPLE_CHARS));
        assert!(before.starts_with('…') && before.ends_with("пример."), "an end mark keeps the text's end: {before}");
    }

    #[test]
    fn debug_shows_no_words() {
        let c = correction("Здравствуйте, Анна!", "Добрый день, Анна!");
        let shown = format!("{c:?} {:?} {:?}", c.found[0], c.found[0].form);
        for word in ["Анна", "добрый", "здравствуйте"] {
            assert!(!shown.contains(word), "{word} in {shown}");
        }
    }

    const TG: &str = "Telegram Desktop";

    fn learned(h: &mut Habits, pasted: &str, corrected: &str, lang: &str) -> Habit {
        let c = correction(pasted, corrected);
        let mut draft = None;
        for _ in 0..LEARN_AT {
            draft = h.count(TG, &c, "2026-11-04", lang).new;
        }
        h.adopt(draft.expect("the third time"), None, lang)
    }

    #[test]
    fn three_times_in_one_app_make_one_habit_of_the_forms_together() {
        let mut h = habits();
        let c = correction("Скинь файл.", "скинь файл");
        assert_eq!(h.count(TG, &c, "2026-11-02", "ru"), Counted::default());
        assert_eq!(h.seen.len(), 2);
        assert!(h.seen.iter().all(|s| s.n == 1 && s.app == TG && s.last == "2026-11-02"));
        assert_eq!(h.count(TG, &c, "2026-11-03", "ru").new, None);
        let draft = h.count(TG, &c, "2026-11-04", "ru").new.expect("the third time");
        assert_eq!(draft.forms, [Form::FirstLetter { lower: true }, STOP]);
        assert_eq!((draft.app.as_str(), draft.before.as_str(), draft.after.as_str(), draft.n), (TG, "Скинь файл.", "скинь файл", 3));
        assert!(h.seen.is_empty(), "the counts went into the habit");
        let habit = h.adopt(draft, None, "ru");
        assert_eq!(habit.rule, "В Telegram Desktop: с маленькой буквы и без точки в конце");
        assert_eq!(habit.short, "без точки");
        assert_eq!(habit.id, "hb7a43023672e208b", "made from its forms and its app");
        assert_eq!(id_of(&habit.forms, None, &h.secret), "hca6cdd4168133ac4", "the same forms everywhere");
        assert_eq!((habit.app.as_deref(), habit.n, habit.on), (Some(TG), 3, true));
        assert_eq!(h.list, [habit]);
    }

    #[test]
    fn without_a_secret_nothing_is_counted() {
        let mut h = Habits { secret: Secret::default(), ..Habits::default() };
        let c = correction("Скинь файл.", "скинь файл");
        for _ in 0..LEARN_AT {
            assert_eq!(h.count(TG, &c, "2026-11-04", "en"), Counted::default());
        }
        assert!(h.seen.is_empty());
    }

    /// Without the secret a forgotten habit goes, but its forms are not declined: a hash under no secret is easy to
    /// guess, and it would match nothing once the secret is there.
    #[test]
    fn without_a_secret_a_forgotten_habit_goes_and_nothing_is_declined() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.secret = Secret::default();
        assert!(h.forget(&id));
        assert!(h.list.is_empty() && h.declined.is_empty(), "{h:?}");
        assert!(!h.forget(&id), "already gone");
    }

    /// The secret was missing or malformed, and a new one was made: the counts and the declined forms go (their hashes
    /// no longer match), the habits keep working with their ids made again.
    #[test]
    fn a_new_secret_drops_the_counts_and_the_declined_forms_and_keeps_the_habits() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count(TG, &correction("клиенты - в основном", "клиенты — в основном"), "2026-11-04", "en");
        h.declined.push(Form::Digits.hash(&h.secret));
        assert_eq!(h.rekey(key(9)), (1, 1));
        assert!(h.seen.is_empty() && h.declined.is_empty());
        assert_eq!(h.secret, key(9));
        let id = id_of(&[Form::FirstLetter { lower: true }, STOP], Some(TG), &key(9));
        assert_ne!(id, old);
        assert_eq!(h.list[0].id, id, "its id is made with the new secret");
        assert_eq!(h.for_app(TG).count(), 1, "still applied");
        assert!(h.count(TG, &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en").grown, "and counted on");
        assert!(h.forget(&id));
        assert_eq!(h.declined, [Form::FirstLetter { lower: true }.hash(&key(9)), STOP.hash(&key(9))]);
    }

    #[test]
    fn other_apps_count_apart_and_an_unknown_app_not_at_all() {
        let mut h = habits();
        let c = correction("клиенты - в основном", "клиенты — в основном");
        h.count(TG, &c, "2026-11-04", "en");
        h.count("Word", &c, "2026-11-04", "en");
        h.count(TG, &c, "2026-11-04", "en");
        assert_eq!(h.seen.iter().map(|s| (s.app.as_str(), s.n)).collect::<Vec<_>>(), [(TG, 2), ("Word", 1)]);
        assert_eq!(h.count("", &c, "2026-11-04", "en"), Counted::default());
        assert_eq!(h.seen.len(), 2);
    }

    #[test]
    fn counts_older_than_thirty_days_go_and_at_most_three_hundred_stay() {
        let mut h = habits();
        let (dash, digits) = (Form::Dash.hash(&h.secret), Form::Digits.hash(&h.secret));
        h.seen.push(Seen { app: TG.into(), form: dash.clone(), n: 2, last: "2026-10-04".into() });
        h.seen.push(Seen { app: TG.into(), form: digits.clone(), n: 2, last: "2026-10-05".into() });
        h.count(TG, &correction("клиенты - в основном", "клиенты — в основном"), "2026-11-04", "en");
        let left: Vec<(&str, u32)> = h.seen.iter().map(|s| (s.form.as_str(), s.n)).collect();
        assert_eq!(left, [(dash.as_str(), 1), (digits.as_str(), 2)], "the dash's old count went; the digits' stays");
        for i in 0..400 {
            h.seen.push(Seen { app: format!("app{i}"), form: dash.clone(), n: 1, last: "2026-11-04".into() });
        }
        h.count("Word", &correction("рост двенадцать процентов", "рост 12 %"), "2026-11-04", "en");
        assert_eq!(h.seen.len(), MAX_SEEN);
        assert_eq!((h.seen[0].app.as_str(), h.seen[0].form.as_str()), ("Word", digits.as_str()), "the newest first");
    }

    /// A habit of one form: the form alone is its whole set.
    #[test]
    fn a_habit_of_one_form_widens_when_it_comes_three_times_in_another_app() {
        let mut h = habits();
        let c = correction("клиенты - в основном", "клиенты — в основном");
        let mut draft = None;
        for _ in 0..LEARN_AT {
            draft = h.count(TG, &c, "2026-11-04", "ru").new;
        }
        let rule = RuleText { rule: "В Telegram Desktop — тире вместо дефиса".into(), short: "тире".into() };
        let id = h.adopt(draft.unwrap(), Some(rule), "ru").id;
        h.count("Word", &c, "2026-11-04", "ru");
        h.count("Word", &c, "2026-11-04", "ru");
        let counted = h.count("Word", &c, "2026-11-04", "ru");
        let all = id_of(&[Form::Dash], None, &h.secret);
        assert_eq!((counted.new, counted.widened), (None, vec![(id, all.clone())]));
        let habit = &h.list[0];
        assert_eq!((habit.app.as_deref(), habit.n, habit.rule.as_str()), (None, 6, "Тире вместо дефиса"));
        assert_eq!(habit.id, all, "the id of everywhere");
        assert!(h.seen.is_empty());
        assert!(h.count("Notepad", &c, "2026-11-04", "ru").grown, "everywhere now: a further correction counts on it");
        assert_eq!(h.list[0].n, 7);
    }

    #[test]
    fn a_rule_that_does_not_name_the_app_is_written_anew_from_the_template() {
        assert_eq!(unscoped("In Telegram Desktop: no full stop at the end", TG, &[STOP], "en"), "No full stop at the end");
        assert_eq!(unscoped("В Telegram Desktop — с маленькой буквы", TG, &[STOP], "ru"), "С маленькой буквы");
        assert_eq!(unscoped("Без точки, как вы пишете", TG, &[STOP], "ru"), "Без точки в конце");
    }

    #[test]
    fn a_forgotten_habit_is_never_learned_again_until_all_is_forgotten() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        assert!(h.forget(&id));
        assert!(!h.forget(&id), "already gone");
        assert_eq!(h.declined, [Form::FirstLetter { lower: true }.hash(&h.secret), STOP.hash(&h.secret)]);
        let c = correction("Скинь файл.", "скинь файл");
        for _ in 0..5 {
            assert_eq!(h.count(TG, &c, "2026-11-04", "en"), Counted::default());
        }
        assert!(h.seen.is_empty(), "a declined form is not even counted");
        assert!(h.forget_all());
        assert_eq!(h, habits(), "the secret stays");
        assert!(!h.forget_all(), "nothing left");
    }

    #[test]
    fn a_habit_switched_off_stays_but_goes_into_no_request() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        assert_eq!(h.for_app(TG).count(), 1);
        assert_eq!(h.for_app("Word").count(), 0, "another app's");
        assert_eq!(h.for_app("").count(), 0, "an unknown app gets only the habits of everywhere");
        assert!(h.switch(&id, false));
        assert!(!h.switch(&id, false), "no change");
        assert_eq!(h.for_app(TG).count(), 0);
        assert!(!h.switch("nope", true));
    }

    #[test]
    fn at_most_fifty_habits_the_newest_first() {
        let mut h = habits();
        for i in 0..MAX_HABITS + 2 {
            let forms = vec![Form::Replacement { from: format!("a{i}"), to: format!("b{i}") }];
            h.adopt(Draft { forms, app: TG.into(), before: format!("a{i}"), after: format!("b{i}"), n: 3 }, None, "en");
        }
        assert_eq!(h.list.len(), MAX_HABITS);
        assert_eq!(h.list[0].after, format!("b{}", MAX_HABITS + 1));
    }

    #[test]
    fn templates_say_the_rule_and_its_short_name() {
        let t = |forms: &[Form], app: Option<&str>, lang: &str| {
            let r = template(forms, app, lang);
            (r.rule, r.short)
        };
        assert_eq!(t(&[STOP], Some("Telegram"), "en"), ("In Telegram: no full stop at the end".into(), "no full stop".into()));
        assert_eq!(t(&[STOP], Some("Telegram"), "ru"), ("В Telegram: без точки в конце".into(), "без точки".into()));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert_eq!(t(&[hello], None, "ru"), ("Писать «добрый день» вместо «здравствуйте»".into(), "добрый день".into()));
        assert_eq!(t(&[Form::Digits], None, "en").0, "Numbers in digits");
        assert_eq!(t(&[Form::Dash], Some("Docs"), "ru").0, "В Docs: тире вместо дефиса");
        assert_eq!(
            t(&[Form::FirstLetter { lower: true }, STOP], Some("Telegram"), "en").0,
            "In Telegram: lower case at the start and no full stop at the end",
        );
        let long = Form::Replacement { from: "я".repeat(40), to: "ю".repeat(40) };
        let r = template(&[long], Some("Telegram"), "ru");
        assert!(r.rule.chars().count() <= MAX_RULE_CHARS && r.short.chars().count() <= MAX_SHORT_CHARS);
    }

    #[test]
    fn the_rule_request_holds_the_kind_the_example_and_the_app_only() {
        let forms = vec![Form::FirstLetter { lower: true }, STOP];
        let d = Draft { forms, app: "Telegram".into(), before: "Скинь файл.".into(), after: "скинь файл".into(), n: 3 };
        let (system, user) = rule_messages(&d.ask(), "ru");
        let v: serde_json::Value = serde_json::from_str(&user).unwrap();
        let expected = serde_json::json!({ "kind": ["firstLetter", "endMark"], "before": "Скинь файл.", "after": "скинь файл", "app": "Telegram" });
        assert_eq!(v, expected);
        assert!(system.contains("Write both in Russian.") && !system.contains("Скинь") && !system.contains("скинь"));
        assert!(rule_messages(&d.ask(), "en").0.contains("Write both in English."));
    }

    #[test]
    fn only_a_usable_answer_is_taken() {
        let ok = read_rule(r#"{"rule":"В Telegram — с маленькой буквы и без точки.","short":"без точки"}"#).unwrap();
        assert_eq!((ok.rule.as_str(), ok.short.as_str()), ("В Telegram — с маленькой буквы и без точки", "без точки"));
        assert!(read_rule("```json\n{\"rule\":\"No full stop\",\"short\":\"no stop\"}\n```").is_some(), "in a code fence");
        let long = format!(r#"{{"rule":"{}","short":"x"}}"#, "a".repeat(MAX_RULE_CHARS + 1));
        for bad in [
            r#"{"rule":"","short":"x"}"#,
            long.as_str(),
            r#"{"rule":"two\nlines","short":"x"}"#,
            r#"{"rule":"fine","short":"one two three four"}"#,
            "not json",
            r#"{"text":"fine"}"#,
        ] {
            assert!(read_rule(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_applied_check_per_kind() {
        assert!(STOP.shows_in("скинь файл") && !STOP.shows_in("Скинь файл."));
        assert!(Form::EndMark { mark: '.', removed: false }.shows_in("Готово."));
        let lower = Form::FirstLetter { lower: true };
        assert!(lower.shows_in("«скинь» файл") && !lower.shows_in("Скинь файл"));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert!(hello.shows_in("Добрый день, Анна!"));
        assert!(!hello.shows_in("Здравствуйте, Анна!") && !hello.shows_in("Добрый день! Здравствуйте, Анна"));
        assert!(Form::Digits.shows_in("рост 12 %") && !Form::Digits.shows_in("рост двенадцать процентов"));
        assert!(Form::Dash.shows_in("клиенты — в основном") && !Form::Dash.shows_in("клиенты - в основном"));
    }

    #[test]
    fn only_habits_whose_every_form_shows_are_marked() {
        let habit = |id: &str, forms: Vec<Form>| Habit {
            id: id.into(),
            forms,
            rule: "r".into(),
            short: format!("s{id}"),
            before: String::new(),
            after: String::new(),
            app: None,
            n: 3,
            on: true,
        };
        let sent = [habit("1", vec![Form::FirstLetter { lower: true }, STOP]), habit("2", vec![Form::Digits])];
        assert_eq!(marks(&sent, "Скинь файл.", "скинь файл"), [HabitMark { id: "1".into(), short: "s1".into() }]);
        assert!(marks(&sent, "Скинь файл.", "Скинь файл").is_empty());
    }

    /// A habit goes on the entry only when the cleanup brought its effect: what was heard may have had it already.
    #[test]
    fn a_habit_whose_effect_the_text_as_heard_had_already_is_not_marked() {
        let habit = |id: &str, form: Form| Habit {
            id: id.into(),
            forms: vec![form],
            rule: "r".into(),
            short: format!("s{id}"),
            before: String::new(),
            after: String::new(),
            app: None,
            n: 3,
            on: true,
        };
        let digits = [habit("d", Form::Digits)];
        assert_eq!(marks(&digits, "рост двенадцать процентов", "рост 12 %").len(), 1);
        assert!(marks(&digits, "рост 12 %", "рост 12 %").is_empty(), "the recognizer wrote the digits");
        let stop = [habit("e", STOP)];
        assert_eq!(marks(&stop, "Скинь файл.", "скинь файл").len(), 1);
        assert!(marks(&stop, "скинь файл", "скинь файл").is_empty(), "heard without a full stop");
    }

    #[test]
    fn the_lists_read_leniently_and_print_no_text() {
        let json = serde_json::json!({
            "seen": [{ "app": "Telegram Desktop", "form": "94128fbe83ca55c4", "n": 2, "last": "2026-11-04" }, { "app": 1 }],
            "list": [
                { "id": "h1", "forms": [{ "kind": "endMark", "mark": ".", "removed": true }], "rule": "В Telegram: без точки в конце",
                  "short": "без точки", "before": "Скинь файл.", "after": "скинь файл", "app": "Telegram Desktop", "n": 3 },
                { "id": "h2", "forms": [{ "kind": "later" }] }
            ],
            "declined": ["85657e6730abe435", 7]
        });
        let h: Habits = serde_json::from_value(json).unwrap();
        assert_eq!((h.seen.len(), h.list.len(), h.declined.len()), (1, 1, 1));
        assert!(h.list[0].on, "on unless written off");
        let shown = format!("{h:?}");
        for word in ["Скинь", "скинь", "Telegram", "точки"] {
            assert!(!shown.contains(word), "{word} in {shown}");
        }
        let h: Habits = serde_json::from_value(serde_json::json!({ "seen": 5, "list": "x" })).unwrap();
        assert_eq!(h, Habits::default());
        assert_eq!(serde_json::to_value(Habits::default()).unwrap(), serde_json::json!({}));
    }

    #[test]
    fn a_habit_of_two_forms_grows_by_one_per_correction_and_widening_adds_only_its_count() {
        let mut h = habits();
        learned(&mut h, "Скинь файл.", "скинь файл", "en");
        let c = correction("Скинь файл.", "скинь файл");
        assert!(h.count(TG, &c, "2026-11-04", "en").grown);
        assert_eq!(h.list[0].n, 4);
        for _ in 0..2 {
            h.count("Word", &c, "2026-11-04", "en");
        }
        let counted = h.count("Word", &c, "2026-11-04", "en");
        assert_eq!(counted.widened.len(), 1);
        assert!(!counted.grown && counted.new.is_none());
        assert_eq!(h.list[0].n, 4 + 3, "only the widening's count, once");
        assert_eq!(h.list[0].app, None);
        assert!(h.seen.is_empty(), "every count of its forms went, in every app");
    }

    #[test]
    fn widening_removes_the_counts_of_its_set_and_its_forms_in_every_app() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count("Notepad", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        h.count("Notepad", &correction("Скинь файл.", "Скинь файл"), "2026-11-04", "en");
        let seen: Vec<(&str, &str, u32)> = h.seen.iter().map(|s| (s.app.as_str(), s.form.as_str(), s.n)).collect();
        let stop = STOP.hash(&h.secret);
        assert_eq!(seen, [("Notepad", stop.as_str(), 1), ("Notepad", id.as_str(), 1)], "the set under the habit's id");
        let c = correction("Скинь файл.", "скинь файл");
        h.count("Word", &c, "2026-11-04", "en");
        h.count("Word", &c, "2026-11-04", "en");
        let counted = h.count("Word", &c, "2026-11-04", "en");
        let all = id_of(&[Form::FirstLetter { lower: true }, STOP], None, &h.secret);
        assert_eq!(counted.widened, [(id, all.clone())]);
        assert_eq!(h.list[0].id, all);
        assert!(h.seen.is_empty(), "the sets under the id it had go, none follows it: {:?}", h.seen);
    }

    /// Widening asks for the habit's whole set in one dictation: its forms seen apart in the other app count for that
    /// app, as any form does.
    #[test]
    fn all_its_forms_together_three_times_in_another_app_widen_a_habit() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        let both = correction("Скинь файл.", "скинь файл");
        let stop = correction("Скинь файл.", "Скинь файл");
        h.count("Word", &both, "2026-11-04", "en");
        h.count("Word", &stop, "2026-11-04", "en");
        h.count("Word", &both, "2026-11-04", "en");
        let seen: Vec<(&str, u32)> = h.seen.iter().map(|s| (s.form.as_str(), s.n)).collect();
        assert_eq!(seen, [(id.as_str(), 2), (STOP.hash(&h.secret).as_str(), 1)], "the set twice, the full stop alone once");
        assert_eq!(h.list[0].app.as_deref(), Some(TG));
        let counted = h.count("Word", &both, "2026-11-04", "en");
        let all = id_of(&[Form::FirstLetter { lower: true }, STOP], None, &h.secret);
        assert_eq!((counted.widened, counted.new), (vec![(id, all)], None));
        let habit = &h.list[0];
        assert_eq!((habit.app.as_deref(), habit.n, habit.rule.as_str()), (None, 6, "Lower case at the start and no full stop at the end"));
        assert!(h.seen.is_empty());
    }

    /// One form of a habit, three times alone in another app: no widening, but that app's own habit.
    #[test]
    fn one_form_of_a_habit_alone_in_another_app_makes_that_apps_own_habit() {
        let mut h = habits();
        let first = learned(&mut h, "Скинь файл.", "скинь файл", "en");
        let stop = correction("Скинь файл.", "Скинь файл");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &stop, "2026-11-04", "en");
        }
        assert!(counted.widened.is_empty() && !counted.grown);
        let draft = counted.new.expect("Word's own");
        assert_eq!((draft.forms.as_slice(), draft.app.as_str()), ([STOP].as_slice(), "Word"));
        let word = h.adopt(draft, None, "en");
        assert_eq!(h.list, [word.clone(), first.clone()], "the first stays in its app");
        assert_eq!(word.rule, "In Word: no full stop at the end");
        assert_eq!(h.for_app("Word").map(|x| x.id.as_str()).collect::<Vec<_>>(), [word.id.as_str()]);
        assert_eq!(h.for_app(TG).map(|x| x.id.as_str()).collect::<Vec<_>>(), [first.id.as_str()]);
        // In Word now, the full stop counts on Word's habit and the first letter alone; the set is not whole.
        let counted = h.count("Word", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        assert!(counted.grown && counted.widened.is_empty());
        assert_eq!(h.list[0].n, 4);
        assert_eq!(h.seen.iter().map(|s| s.form.clone()).collect::<Vec<_>>(), [Form::FirstLetter { lower: true }.hash(&h.secret)]);
    }

    #[test]
    fn a_forgotten_habit_takes_its_sets_counts_along() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        h.count("Word", &correction("Скинь файл.", "скинь файл"), "2026-11-04", "en");
        assert_eq!(h.seen.len(), 1);
        assert!(h.forget(&id));
        assert!(h.seen.is_empty());
    }

    const ADDED: Form = Form::EndMark { mark: '.', removed: false };

    #[test]
    fn each_form_has_its_opposite_but_digits_and_a_dash() {
        assert_eq!((STOP.opposite(), ADDED.opposite()), (Some(ADDED), Some(STOP)));
        assert_eq!(Form::FirstLetter { lower: true }.opposite(), Some(Form::FirstLetter { lower: false }));
        let hello = Form::Replacement { from: "здравствуйте".into(), to: "добрый день".into() };
        assert_eq!(hello.opposite(), Some(Form::Replacement { from: "добрый день".into(), to: "здравствуйте".into() }));
        assert_eq!((Form::Digits.opposite(), Form::Dash.opposite()), (None, None));
    }

    /// The user goes back on a habit: the opposite counts like any form, and at three the habit changes. The old one
    /// goes without being declined; "Undo" brings it back as it was and declines the new forms.
    #[test]
    fn an_opposite_three_times_changes_the_habit_and_undo_brings_it_back() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        assert_eq!(kinds("Скинь файл", "Скинь файл."), [ADDED]);
        let back = correction("Скинь файл", "Скинь файл.");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count(TG, &back, "2026-11-04", "en");
            assert!(!counted.grown, "the habit's own count does not grow");
        }
        let changed = h.learn(counted.new.expect("the opposite at three"), None, "en");
        assert_eq!((changed.replaced.as_slice(), changed.merged), ([old.clone()].as_slice(), false));
        assert_eq!((changed.habit.rule.as_str(), changed.habit.app.as_deref()), ("In Telegram Desktop: a full stop at the end", Some(TG)));
        assert_eq!(h.list, std::slice::from_ref(&changed.habit), "the two never live together");
        assert!(h.declined.is_empty(), "the old habit may be learned again");
        assert_eq!(h.revert(&changed.habit.id, changed.replaced, true), Some(Vec::new()));
        assert_eq!(h.list, [old]);
        assert_eq!(h.declined, [ADDED.hash(&h.secret)]);
        assert_eq!(h.count(TG, &back, "2026-11-04", "en"), Counted::default(), "the new form is declined");
        assert_eq!(h.revert("nope", Vec::new(), true), None);
    }

    /// An opposite in another app is that app's own habit: the first one is not changed.
    #[test]
    fn an_opposite_in_another_app_leaves_the_habit_alone() {
        let mut h = habits();
        let old = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        let back = correction("Скинь файл", "Скинь файл.");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &back, "2026-11-04", "en");
        }
        let word = h.learn(counted.new.expect("Word's own"), None, "en");
        assert!(word.replaced.is_empty());
        assert_eq!(h.list, [word.habit, old]);
    }

    /// A habit of everywhere is changed by an opposite reaching three in one app, and the change holds everywhere: the
    /// app habits it goes back on go too. Without `decline`, reverting leaves the new forms free.
    #[test]
    fn a_change_of_a_habit_of_everywhere_holds_everywhere() {
        let mut h = habits();
        let draft = |forms: Vec<Form>, app: &str| Draft { forms, app: app.into(), before: "a".into(), after: "b".into(), n: 3 };
        h.adopt(draft(vec![Form::FirstLetter { lower: true }], TG), None, "en");
        h.list[0].app = None; // as if it had widened
        h.adopt(draft(vec![STOP], "Notepad"), None, "en");
        let dash = h.adopt(draft(vec![Form::Dash], "Word"), None, "en");
        let before = h.list.clone();
        let back = correction("скинь файл", "Скинь файл.");
        assert_eq!(back.found.iter().map(|f| f.form.clone()).collect::<Vec<_>>(), [Form::FirstLetter { lower: false }, ADDED]);
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Word", &back, "2026-11-04", "en");
        }
        let changed = h.learn(counted.new.expect("in Word"), None, "en");
        assert_eq!(changed.replaced, before[1..], "the full stop of Notepad and the lower case of everywhere");
        assert_eq!((changed.habit.app.as_deref(), changed.habit.rule.as_str()), (None, "A capital at the start and a full stop at the end"));
        assert_eq!(h.list, [changed.habit.clone(), dash]);
        assert_eq!(h.revert(&changed.habit.id, changed.replaced, false), Some(Vec::new()));
        assert!(h.declined.is_empty());
        let mut back_again = h.list.clone();
        let mut was = before.clone();
        back_again.sort_by(|a, b| a.id.cmp(&b.id));
        was.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(back_again, was, "the three as they were");
    }

    /// A habit with an opposite habit in another app never holds everywhere, where the two would live together:
    /// its forms count for a third app on their own and make that app's own habit, which leaves the first alone.
    #[test]
    fn a_habit_with_an_opposite_in_another_app_never_widens() {
        let mut h = habits();
        let tg = learned(&mut h, "Скинь файл.", "Скинь файл", "en");
        let word = Draft { forms: vec![ADDED], app: "Word".into(), before: "a".into(), after: "a.".into(), n: 3 };
        let word = h.adopt(word, None, "en");
        let stop = correction("Скинь файл.", "Скинь файл");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Notepad", &stop, "2026-11-04", "en");
        }
        assert!(counted.widened.is_empty());
        let notepad = h.learn(counted.new.expect("its form counted for Notepad"), None, "en");
        assert!(!notepad.merged && notepad.replaced.is_empty() && notepad.was.is_none());
        assert_eq!((notepad.habit.app.as_deref(), notepad.habit.n), (Some("Notepad"), 3));
        assert_ne!(notepad.habit.id, tg.id);
        assert_eq!(h.list, [notepad.habit, word, tg], "Telegram's habit as it was");
    }

    /// Two apps' drafts of the same forms make two habits, one in each app, each with its own id: a draft never adds
    /// to a habit of another app. The same app's second draft adds to its habit. Their set coming together three times
    /// in a third app makes them one habit of everywhere, with the id of everywhere.
    #[test]
    fn two_apps_habits_of_the_same_forms_live_apart_with_their_own_ids() {
        let mut h = habits();
        let c = correction("клиенты - в основном", "клиенты — в основном");
        let (mut in_tg, mut in_word) = (None, None);
        for _ in 0..LEARN_AT {
            in_tg = h.count(TG, &c, "2026-11-04", "ru").new;
            in_word = h.count("Word", &c, "2026-11-04", "ru").new;
        }
        let (in_tg, in_word) = (in_tg.expect("Telegram's draft"), in_word.expect("Word's draft"));
        let tg = h.learn(in_tg, Some(RuleText { rule: "В Telegram Desktop — тире вместо дефиса".into(), short: "тире".into() }), "ru");
        let word = h.learn(in_word, Some(RuleText { rule: "В Word — тире вместо дефиса".into(), short: "тире".into() }), "ru");
        assert!(!tg.merged && !word.merged);
        let (tg, word) = (tg.habit, word.habit);
        assert_ne!(tg.id, word.id);
        assert_eq!((tg.id.clone(), word.id.clone()), (id_of(&[Form::Dash], Some(TG), &h.secret), id_of(&[Form::Dash], Some("Word"), &h.secret)));
        assert_eq!(h.list, [word.clone(), tg.clone()]);
        assert_eq!((tg.app.as_deref(), tg.n, word.app.as_deref(), word.n), (Some(TG), 3, Some("Word"), 3));
        assert_eq!(h.for_app("Word").map(|x| x.id.as_str()).collect::<Vec<_>>(), [word.id.as_str()]);
        // The same app again: the count adds up and the app stays.
        let d = Draft { forms: vec![Form::Dash], app: TG.into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        let again = h.learn(d, None, "ru");
        assert!(again.merged && again.replaced.is_empty());
        assert_eq!(again.was.as_ref(), Some(&tg));
        assert_eq!((again.habit.id.as_str(), again.habit.app.as_deref(), again.habit.n), (tg.id.as_str(), Some(TG), 6));
        // Everywhere: the two fold into one, and their counts add up.
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count("Notepad", &c, "2026-11-04", "ru");
        }
        let all = id_of(&[Form::Dash], None, &h.secret);
        assert_eq!(counted.widened, [(word.id.clone(), all.clone()), (tg.id.clone(), all.clone())]);
        assert_eq!(h.list.len(), 1);
        assert_eq!((h.list[0].id.as_str(), h.list[0].app.as_deref(), h.list[0].n), (all.as_str(), None, 3 + 3 + 6));
        assert!(h.seen.is_empty(), "{:?}", h.seen);
    }

    /// A draft of the forms of a habit of everywhere (it widened while the draft waited for its rule) adds to it.
    #[test]
    fn a_draft_adds_to_the_habit_of_everywhere_of_its_forms() {
        let mut h = habits();
        let d = |app: &str| Draft { forms: vec![Form::Dash], app: app.into(), before: "a - b".into(), after: "a — b".into(), n: 3 };
        h.adopt(d("Word"), None, "en");
        h.list[0].app = None; // as if it had widened
        assert!(h.reid());
        let all = h.list[0].clone();
        assert_eq!(all.id, id_of(&[Form::Dash], None, &h.secret));
        let m = h.learn(d(TG), None, "en");
        assert!(m.merged && m.replaced.is_empty());
        assert_eq!(m.was.as_ref(), Some(&all));
        assert_eq!((m.habit.id.as_str(), m.habit.app, m.habit.n), (all.id.as_str(), None, 6));
        assert_eq!(h.revert(&m.habit.id, m.was.into_iter().collect(), false), Some(Vec::new()));
        assert_eq!(h.list, [all], "taken back without a trace");
        assert!(h.declined.is_empty());
    }

    /// A settings.json restored or brought over may hold ids made otherwise: they are made again under the secret, and
    /// a set counted under a habit's id follows it.
    #[test]
    fn ids_made_otherwise_are_made_again_and_their_sets_follow() {
        let mut h = habits();
        let id = learned(&mut h, "Скинь файл.", "скинь файл", "en").id;
        assert!(!h.reid(), "every id is made under the secret already");
        h.list[0].id = "h1".into();
        h.seen.push(Seen { app: "Word".into(), form: "h1".into(), n: 2, last: "2026-11-04".into() });
        h.seen.push(Seen { app: "Word".into(), form: STOP.hash(&h.secret), n: 1, last: "2026-11-04".into() });
        assert!(h.reid());
        assert_eq!(h.list[0].id, id);
        assert_eq!(h.seen.iter().map(|s| s.form.clone()).collect::<Vec<_>>(), [id, STOP.hash(&h.secret)]);
        assert!(!h.reid());
    }

    /// "Undo" wins over a habit learned since in the app that goes back on the habit it brings back: that one goes,
    /// not declined, and is reported.
    #[test]
    fn an_undo_takes_out_a_habit_learned_since_that_opposes_the_one_it_brings_back() {
        let mut h = habits();
        let r = learned(&mut h, "Скинь файл.", "скинь файл", "en");
        let three = |h: &mut Habits, app: &str, pasted: &str, corrected: &str| {
            let c = correction(pasted, corrected);
            let mut counted = Counted::default();
            for _ in 0..LEARN_AT {
                counted = h.count(app, &c, "2026-11-04", "en");
            }
            counted
        };
        let c = three(&mut h, TG, "скинь файл", "скинь файл.").new.expect("the full stop back");
        let c = h.learn(c, None, "en");
        assert_eq!(c.replaced, std::slice::from_ref(&r));
        let d = three(&mut h, TG, "скинь файл.", "Скинь файл.").new.expect("a capital");
        let d = h.learn(d, None, "en");
        assert!(d.replaced.is_empty(), "the capital goes back on nothing listed");
        assert_eq!(h.list, [d.habit.clone(), c.habit.clone()]);
        assert_eq!(h.revert(&c.habit.id, c.replaced, true), Some(vec![d.habit]));
        assert_eq!(h.list, [r]);
        assert_eq!(h.declined, [ADDED.hash(&h.secret)], "only the undone change's forms");
    }

    /// A habit of another app held back by the one replaced widens once it is gone; "Undo" brings the first back and
    /// takes the widened one out.
    #[test]
    fn an_undo_takes_out_a_habit_widened_since_that_opposes_the_one_it_brings_back() {
        let mut h = habits();
        let r = learned(&mut h, "Скинь файл.", "скинь файл", "en");
        let w = Draft { forms: vec![ADDED], app: "Word".into(), before: "a".into(), after: "a.".into(), n: 3 };
        let w = h.adopt(w, None, "en");
        let capital = correction("скинь файл", "Скинь файл");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count(TG, &capital, "2026-11-04", "en");
        }
        let c = h.learn(counted.new.expect("a capital in Telegram"), None, "en");
        assert_eq!(c.replaced, std::slice::from_ref(&r));
        let stop = correction("Скинь файл", "Скинь файл.");
        for _ in 0..LEARN_AT {
            counted = h.count("Notepad", &stop, "2026-11-04", "en");
        }
        let all = id_of(&[ADDED], None, &h.secret);
        assert_eq!(counted.widened, [(w.id.clone(), all.clone())], "nothing opposes Word's habit any more");
        let widened = h.list.iter().find(|x| x.id == all).cloned().unwrap();
        assert_eq!(widened.app, None);
        assert_eq!(h.revert(&c.habit.id, c.replaced, true), Some(vec![widened]));
        assert_eq!(h.list, [r]);
    }

    /// A merged draft taken back without a trace: the habit it added to comes back as it was, the other app's habit of
    /// the opposite form untouched.
    #[test]
    fn a_merged_draft_taken_back_leaves_the_habit_it_added_to_as_it_was() {
        let mut h = habits();
        let draft = |forms: Vec<Form>, app: &str| Draft { forms, app: app.into(), before: "a".into(), after: "b".into(), n: 3 };
        let x = h.adopt(draft(vec![STOP], TG), None, "en");
        let y = h.adopt(draft(vec![ADDED], "Word"), None, "en");
        let m = h.learn(draft(vec![STOP], TG), None, "en");
        assert!(m.merged && m.replaced.is_empty());
        assert_eq!(m.was.as_ref(), Some(&x));
        assert_eq!((m.habit.id.as_str(), m.habit.app.as_deref(), m.habit.n), (x.id.as_str(), Some(TG), 6), "it stays in its app");
        let back: Vec<Habit> = m.was.into_iter().chain(m.replaced).collect();
        assert_eq!(h.revert(&m.habit.id, back, false), Some(Vec::new()));
        assert_eq!(h.list, [x, y]);
        assert!(h.declined.is_empty());
    }

    /// Changes taken back newest first: a change of a habit itself a change waiting comes back before it, and the
    /// first habit is listed again. Oldest first, the first change is not there to take back.
    #[test]
    fn changes_are_taken_back_newest_first() {
        let mut h = habits();
        let r = learned(&mut h, "Скинь файл", "скинь файл", "en");
        let mut three = |pasted: &str, corrected: &str| {
            let c = correction(pasted, corrected);
            let mut counted = Counted::default();
            for _ in 0..LEARN_AT {
                counted = h.count(TG, &c, "2026-11-04", "en");
            }
            let learned = h.learn(counted.new.expect("three times"), None, "en");
            (learned.habit.id, learned.replaced)
        };
        let first = three("скинь файл", "Скинь файл");
        let second = three("Скинь файл.", "скинь файл");
        assert_eq!(first.1, std::slice::from_ref(&r));
        assert_eq!(second.1.len(), 1, "the second changed the first");
        let mut oldest = h.clone();
        assert!(oldest.revert(&first.0, first.1.clone(), false).is_none());
        assert!(oldest.revert(&second.0, second.1.clone(), false).is_some());
        assert_ne!(oldest.list, std::slice::from_ref(&r), "oldest first leaves the first change");
        assert!(h.revert(&second.0, second.1, false).is_some());
        assert!(h.revert(&first.0, first.1, false).is_some());
        assert_eq!(h.list, [r]);
    }

    /// A habit of no forms (a settings.json written by hand) is no set: it never widens, nor counts under its id.
    #[test]
    fn a_habit_of_no_forms_is_no_set() {
        let mut h = habits();
        let d = Draft { forms: Vec::new(), app: "Word".into(), before: "a".into(), after: "b".into(), n: 3 };
        let empty = h.adopt(d, None, "en");
        let stop = correction("Скинь файл.", "Скинь файл");
        let mut counted = Counted::default();
        for _ in 0..LEARN_AT {
            counted = h.count(TG, &stop, "2026-11-04", "en");
        }
        assert!(counted.widened.is_empty(), "{counted:?}");
        assert_eq!(h.list.iter().find(|x| x.id == empty.id).and_then(|x| x.app.as_deref()), Some("Word"));
        assert!(h.seen.iter().all(|s| s.form != empty.id));
        assert!(counted.new.is_some(), "the full stop is Telegram's own");
    }

    #[test]
    fn a_replacement_outside_the_stored_example_is_not_drafted_but_still_counted() {
        let pasted = "здравствуйте коллеги вот большой длинный документ про наши результаты за весь прошлый квартал посмотрите скорее";
        let corrected = "добрый день коллеги вот большой длинный документ про наши результаты за весь прошлый квартал посмотрите быстрее";
        let c = correction(pasted, corrected);
        assert_eq!(c.found.len(), 2, "{:?}", c.found);
        let mut h = habits();
        let mut draft = None;
        for _ in 0..LEARN_AT {
            draft = h.count(TG, &c, "2026-11-04", "en").new;
        }
        let draft = draft.expect("the first replacement");
        assert_eq!(draft.forms.len(), 1, "{draft:?}");
        let Form::Replacement { from, to } = &draft.forms[0] else { panic!("{draft:?}") };
        assert!(draft.before.contains(from.as_str()) && draft.after.contains(to.as_str()));
        assert!(draft.before.chars().count() <= MAX_EXAMPLE_CHARS && draft.after.chars().count() <= MAX_EXAMPLE_CHARS);
        assert_eq!(h.seen.len(), 1, "the far one keeps its count");
        assert_eq!(h.seen[0].n, 3);
    }

    #[test]
    fn an_example_of_several_forms_falls_back_to_a_replacement_and_cuts_at_the_end() {
        let pasted = "здравствуйте коллеги вот большой длинный документ про наши результаты за весь прошлый квартал посмотрите скорее";
        let corrected = "добрый день коллеги вот большой длинный документ про наши результаты за весь прошлый квартал посмотрите быстрее";
        let (before, after) = example(pasted, corrected);
        assert_eq!((before.as_str(), after.as_str()), ("здравствуйте коллеги вот", "добрый день коллеги вот"));
        let (before, after) = example(&format!("{}.", "а".repeat(60)), &"а".repeat(60));
        assert!(before.chars().count() <= MAX_EXAMPLE_CHARS && after.chars().count() <= MAX_EXAMPLE_CHARS);
        assert!(before.starts_with('…') && before.ends_with('.'), "{before}");
    }

    #[test]
    fn an_app_name_is_cut_off_a_rule_only_when_it_is_whole() {
        assert_eq!(unscoped("In Telegram Desktop app — no full stop", "Telegram", &[STOP], "en"), "No full stop at the end");
        assert_eq!(unscoped("In Telegram — no full stop", "Telegram", &[STOP], "en"), "No full stop");
    }
}

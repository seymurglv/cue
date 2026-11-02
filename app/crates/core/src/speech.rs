//! Local speech recognition, the parts that decide (spec "Local speech recognition"): which alphabet a dictation is in,
//! when the other alphabet leaked into it, which language of the translation pair the speech was, and where a long
//! recording is cut into pieces the model takes. Pure logic.
use std::ops::Range;

use crate::translate::{self, Script};

/// The two alphabets a dictation is told apart by: CUE's rule is one language per dictation, Russian or English.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alphabet {
    Cyrillic,
    Latin,
}

impl Alphabet {
    fn of(c: char) -> Option<Self> {
        match translate::script_of(c)? {
            Script::Cyrillic => Some(Self::Cyrillic),
            Script::Latin => Some(Self::Latin),
            _ => None,
        }
    }

    /// Little words a sentence of this alphabet's language needs and a name or a term never holds.
    fn function_words(self) -> &'static [&'static str] {
        match self {
            Self::Latin => &[
                "a", "an", "the", "to", "of", "in", "on", "at", "for", "and", "or", "but", "so", "then", "is", "are",
                "was", "be", "will", "have", "it", "this", "that", "i", "you", "we", "they", "he", "she", "my", "your",
                "do", "not", "please", "with", "if", "when",
            ],
            Self::Cyrillic => &[
                "и", "в", "во", "на", "с", "со", "к", "по", "о", "об", "до", "от", "за", "из", "у", "для", "при", "не",
                "что", "чтобы", "это", "я", "ты", "мы", "вы", "он", "она", "они", "мне", "меня", "тебе", "нам", "его",
                "её", "их", "а", "но", "или", "как", "так", "же", "бы", "ли", "то", "там", "тут", "есть", "если",
                "когда", "будет",
            ],
        }
    }
}

/// Letters of each alphabet in `text`: (Cyrillic, Latin).
pub fn letters(text: &str) -> (usize, usize) {
    text.chars().fold((0, 0), |(c, l), ch| match Alphabet::of(ch) {
        Some(Alphabet::Cyrillic) => (c + 1, l),
        Some(Alphabet::Latin) => (c, l + 1),
        None => (c, l),
    })
}

/// Function words of each language in `text`: (Russian, English). A sentence has them; a name or a term does not.
fn function_words(text: &str) -> (usize, usize) {
    let count = |a: Alphabet| {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty() && w.chars().all(|c| Alphabet::of(c) == Some(a)))
            .filter(|w| a.function_words().contains(&w.to_lowercase().as_str()))
            .count()
    };
    (count(Alphabet::Cyrillic), count(Alphabet::Latin))
}

/// Words of each alphabet in `text`, (Cyrillic, Latin), and the alphabet of the first one: a word belongs to the
/// alphabet of its first letter, so a long foreign name counts once, like a short word.
fn words(text: &str) -> (usize, usize, Option<Alphabet>) {
    let mut first = None;
    let (mut c, mut l) = (0, 0);
    for w in text.split(|ch: char| !ch.is_alphanumeric()) {
        match w.chars().find_map(Alphabet::of) {
            Some(Alphabet::Cyrillic) => c += 1,
            Some(Alphabet::Latin) => l += 1,
            None => continue,
        }
        first = first.or(w.chars().find_map(Alphabet::of));
    }
    (c, l, first)
}

/// The alphabet of the dictation's language: the one whose language's function words it holds more of, so Latin names
/// and terms in a Russian sentence ("запусти cargo test в Visual Studio Code") do not make it English; with as many of
/// each, the one more words are in ("Открой Telegram" is one and one: then the first word's). `None` without letters of
/// either (numbers only, empty).
pub fn alphabet_of(text: &str) -> Option<Alphabet> {
    use std::cmp::Ordering::{Greater, Less};
    let (fc, fl) = function_words(text);
    let (c, l, first) = words(text);
    match (fc.cmp(&fl), c.cmp(&l)) {
        (Greater, _) => Some(Alphabet::Cyrillic),
        (Less, _) => Some(Alphabet::Latin),
        (_, Greater) => Some(Alphabet::Cyrillic),
        (_, Less) => Some(Alphabet::Latin),
        _ => first,
    }
}

/// A leak needs at least this many function words in each alphabet.
pub const LEAK_WORDS: usize = 2;

/// The alphabet to mask out and decode again without, if the model wrote part of the speech in the other one: both
/// alphabets hold at least [`LEAK_WORDS`] function words of their language (two sentences, not a sentence with names),
/// and the one with fewer letters goes. Names and terms ("запусти cargo test", "call Марисоль") never make a leak.
pub fn leak(text: &str) -> Option<Alphabet> {
    let (fc, fl) = function_words(text);
    if fc < LEAK_WORDS || fl < LEAK_WORDS {
        return None;
    }
    let (c, l) = letters(text);
    Some(if c >= l { Alphabet::Latin } else { Alphabet::Cyrillic })
}

/// The speech's language for a translation: the language of the pair (the target first, then home) written in the
/// text's alphabet; `None` when neither is (translation then goes to the target, as for unknown speech).
pub fn speech_language(text: &str, target: &'static str, home: Option<&'static str>) -> Option<&'static str> {
    let script = match alphabet_of(text)? {
        Alphabet::Cyrillic => Script::Cyrillic,
        Alphabet::Latin => Script::Latin,
    };
    [Some(target), home].into_iter().flatten().find(|code| translate::script_of_language(code) == Some(script))
}

/// Samples a second: the model and the recording are 16 kHz mono.
pub const SAMPLE_RATE: usize = 16_000;
/// The longest piece the model gets at once.
pub const MAX_PIECE_S: usize = 60;
/// A cut is looked for no earlier than this into a piece.
pub const MIN_PIECE_S: usize = 45;
/// The quiet window a cut goes in the middle of: 200 ms.
const QUIET_WINDOW: usize = SAMPLE_RATE / 5;
/// The step the quiet window moves by: 50 ms.
const QUIET_STEP: usize = SAMPLE_RATE / 20;

/// While the speaker talks, a piece is recognized once the recording not yet recognized is this long.
pub const EARLY_PIECE_S: usize = 15;
/// An early piece is cut no earlier than this into it.
pub const EARLY_MIN_S: usize = 10;

/// The cut in the middle of the quietest 200 ms of `pcm` that starts between `from` and `to - 200 ms`.
fn quiet_cut(pcm: &[i16], from: usize, to: usize) -> usize {
    let loudness = |at: usize| pcm[at..at + QUIET_WINDOW].iter().map(|s| u64::from(s.unsigned_abs())).sum::<u64>();
    let quietest =
        (from..=to - QUIET_WINDOW).step_by(QUIET_STEP).min_by_key(|&at| loudness(at)).unwrap_or(to - QUIET_WINDOW);
    quietest + QUIET_WINDOW / 2
}

/// Sample ranges of at most [`MAX_PIECE_S`] seconds covering all of `pcm` in order, each cut in the middle of the
/// quietest 200 ms between [`MIN_PIECE_S`] and [`MAX_PIECE_S`] seconds of its piece, so a word is rarely split.
pub fn pieces(pcm: &[i16]) -> Vec<Range<usize>> {
    let (max, min) = (MAX_PIECE_S * SAMPLE_RATE, MIN_PIECE_S * SAMPLE_RATE);
    let mut out = Vec::new();
    let mut start = 0;
    while pcm.len() - start > max {
        let cut = quiet_cut(pcm, start + min, start + max);
        out.push(start..cut);
        start = cut;
    }
    if start < pcm.len() {
        out.push(start..pcm.len());
    }
    out
}

/// Where to cut a piece off the start of `uncut`, the recording not yet recognized, while the speaker still talks:
/// once it is [`EARLY_PIECE_S`] seconds long, in the middle of the quietest 200 ms between [`EARLY_MIN_S`] and
/// [`EARLY_PIECE_S`] seconds; `None` before that.
pub fn early_cut(uncut: &[i16]) -> Option<usize> {
    let (max, min) = (EARLY_PIECE_S * SAMPLE_RATE, EARLY_MIN_S * SAMPLE_RATE);
    (uncut.len() >= max).then(|| quiet_cut(uncut, min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alphabet_is_the_language_of_the_sentence() {
        assert_eq!(alphabet_of("Открой GitHub и проверь."), Some(Alphabet::Cyrillic));
        assert_eq!(alphabet_of("Запусти cargo test в Visual Studio Code."), Some(Alphabet::Cyrillic), "terms, not words");
        assert_eq!(alphabet_of("Check the build, please."), Some(Alphabet::Latin));
        assert_eq!(alphabet_of("Call Марисоль tomorrow."), Some(Alphabet::Latin), "no function words: words decide");
        assert_eq!(alphabet_of("Сониокс"), Some(Alphabet::Cyrillic));
        // A short phrase with a long foreign name: by letters it would be English and go untranslated.
        assert_eq!(alphabet_of("Открой Telegram"), Some(Alphabet::Cyrillic));
        assert_eq!(alphabet_of("Напиши Alexander"), Some(Alphabet::Cyrillic));
        assert_eq!(alphabet_of("Telegram открой"), Some(Alphabet::Latin), "one and one: the first word's");
        assert_eq!(alphabet_of("25 + 17"), None);
        assert_eq!(alphabet_of(""), None);
        assert_eq!(letters("Ёж и Ötzi"), (3, 4), "Ё is Cyrillic, Ö is Latin");
    }

    #[test]
    fn a_leak_is_two_sentences_not_a_sentence_with_names() {
        assert_eq!(leak("Мне нужно, чтобы ты отправил отчёт, and then we will check it"), Some(Alphabet::Latin));
        assert_eq!(leak("Please send the full report to the team, а потом я проверю"), Some(Alphabet::Cyrillic));
        for clean in [
            "Открой GitHub и проверь.",
            "Запусти cargo test в Visual Studio Code.",
            "Call Марисоль tomorrow.",
            "Скажи",
            "The It crowd",
            "123",
            // One function word on the other side is not enough: "the" in a title.
            "Посмотри The Batman сегодня вечером, если будет время",
        ] {
            assert_eq!(leak(clean), None, "{clean}");
        }
    }

    #[test]
    fn the_speech_language_is_the_pair_language_of_its_alphabet() {
        assert_eq!(speech_language("Привет, как дела?", "en", Some("ru")), Some("ru"));
        assert_eq!(speech_language("Hello there", "en", Some("ru")), Some("en"));
        assert_eq!(speech_language("Hallo zusammen", "de", Some("ru")), Some("de"));
        assert_eq!(speech_language("Привет", "ru", Some("en")), Some("ru"), "the target first");
        assert_eq!(speech_language("123", "en", Some("ru")), None);
        assert_eq!(speech_language("Привет", "en", None), None, "no Cyrillic language in the pair");
        assert_eq!(speech_language("Привет", "en", Some("de")), None);
    }

    #[test]
    fn long_audio_is_cut_at_quiet_points() {
        let sr = SAMPLE_RATE;
        assert_eq!(pieces(&vec![1000; 30 * sr]), vec![0..30 * sr]);
        assert_eq!(pieces(&vec![1000; 60 * sr]), vec![0..60 * sr], "exactly the limit is one piece");
        let mut pcm = vec![1000i16; 130 * sr];
        pcm[50 * sr..50 * sr + sr / 5].fill(0);
        let p = pieces(&pcm);
        assert_eq!(p[0], 0..50 * sr + sr / 10, "cut in the middle of the quiet window");
        assert!(p.iter().all(|r| r.len() <= 60 * sr), "{p:?}");
        assert_eq!(p.last().unwrap().end, pcm.len());
        assert!(p.windows(2).all(|w| w[0].end == w[1].start), "no gaps, no overlaps");
        assert!(pieces(&[]).is_empty());
    }

    #[test]
    fn a_recording_is_cut_early_at_a_quiet_point_once_long_enough() {
        let sr = SAMPLE_RATE;
        assert_eq!(early_cut(&vec![1000; 15 * sr - 1]), None, "shorter than the early piece waits");
        assert_eq!(early_cut(&[]), None);
        let mut pcm = vec![1000i16; 15 * sr];
        pcm[12 * sr..12 * sr + sr / 5].fill(0);
        assert_eq!(early_cut(&pcm), Some(12 * sr + sr / 10), "in the middle of the quiet window");
        // Quiet before 10 s does not count: the piece is never shorter.
        let mut pcm = vec![1000i16; 20 * sr];
        pcm[3 * sr..3 * sr + sr / 5].fill(0);
        let cut = early_cut(&pcm).unwrap();
        assert!((10 * sr..=15 * sr).contains(&cut), "{cut}");
    }

    #[test]
    fn without_a_quiet_point_a_piece_is_still_at_most_the_limit() {
        let sr = SAMPLE_RATE;
        let pcm: Vec<i16> = (0..200 * sr).map(|i| if i % 2 == 0 { 3000 } else { -3000 }).collect();
        let p = pieces(&pcm);
        assert!(p.iter().all(|r| r.len() <= 60 * sr && r.len() >= 45 * sr || r.end == pcm.len()), "{p:?}");
        assert_eq!(p.iter().map(|r| r.len()).sum::<usize>(), pcm.len());
    }
}

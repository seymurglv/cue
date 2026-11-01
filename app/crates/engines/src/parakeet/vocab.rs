//! The model's vocabulary (`vocab.txt`: one "token id" per line, `<blk>` the blank) and its text: tokens joined, "▁"
//! a space. Masks of one alphabet keep a dictation in one language (`cue_core::speech::leak`).
use cue_core::speech::Alphabet;

use crate::EngineError;

/// The blank's name in the export.
const BLANK: &str = "<blk>";
/// The SentencePiece word start.
const WORD: char = '▁';

#[derive(Clone, Debug, PartialEq)]
pub struct Vocab {
    tokens: Vec<String>,
    /// The blank's id: the last one in Parakeet's exports.
    pub blank: usize,
}

impl Vocab {
    pub fn parse(text: &str) -> Result<Self, EngineError> {
        let mut tokens: Vec<String> = Vec::new();
        let mut blank = None;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let (token, id) = line
                .rsplit_once(' ')
                .and_then(|(t, id)| Some((t, id.trim().parse::<usize>().ok()?)))
                .ok_or_else(|| EngineError::Model("vocab.txt: a line is not \"token id\"".into()))?;
            if id >= tokens.len() {
                tokens.resize(id + 1, String::new());
            }
            tokens[id] = token.to_owned();
            if token == BLANK {
                blank = Some(id);
            }
        }
        let blank = blank.ok_or_else(|| EngineError::Model("vocab.txt has no blank".into()))?;
        Ok(Self { tokens, blank })
    }

    /// Tokens with the blank: the width of the joint's token logits.
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// The text of `ids`: "▁" starts a word; special tokens (`<…>`) write nothing. Digits glued to a word get a
    /// space ("version 2", not "version2"), as parakeet-rs does: the model often writes a number without its "▁".
    pub fn text(&self, ids: &[usize]) -> String {
        let mut out = String::new();
        for token in ids.iter().filter_map(|&id| self.tokens.get(id)) {
            if special(token) {
                continue;
            }
            let piece = token.replace(WORD, " ");
            if !out.is_empty() && !piece.starts_with(' ') && piece.chars().all(|c| c.is_ascii_digit()) {
                let letters = out.chars().rev().take_while(|c| c.is_alphabetic()).count();
                if letters > 1 || (letters == 1 && out.ends_with('a')) {
                    out.push(' ');
                }
            }
            out.push_str(&piece);
        }
        out.trim().to_owned()
    }

    /// Ids whose text holds a letter of `alphabet`: masked when the dictation leaked into it. Special tokens (`<…>`)
    /// are never masked.
    pub fn mask_of(&self, alphabet: Alphabet) -> Vec<bool> {
        self.tokens.iter().map(|t| !special(t) && cue_core::speech::alphabet_of(t) == Some(alphabet)).collect()
    }
}

fn special(token: &str) -> bool {
    token.starts_with('<') && token.ends_with('>')
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "<unk> 0\n▁При 1\nвет 2\n▁мир 3\n▁the 4\n▁version 5\n2 6\n, 7\n<blk> 8\n";

    #[test]
    fn parses_tokens_and_the_blank() {
        let v = Vocab::parse(SAMPLE).unwrap();
        assert_eq!((v.len(), v.blank), (9, 8));
        assert!(Vocab::parse("▁a 0\n").is_err(), "no blank");
        assert!(Vocab::parse("▁a zero\n<blk> 1").is_err());
    }

    #[test]
    fn text_joins_pieces_into_words() {
        let v = Vocab::parse(SAMPLE).unwrap();
        assert_eq!(v.text(&[1, 2, 7, 3]), "Привет, мир");
        assert_eq!(v.text(&[0, 4, 5, 6, 8]), "the version 2", "special tokens write nothing; a number gets its space");
        assert_eq!(v.text(&[]), "");
    }

    #[test]
    fn masks_hold_the_tokens_of_one_alphabet() {
        let v = Vocab::parse(SAMPLE).unwrap();
        let latin = v.mask_of(Alphabet::Latin);
        assert_eq!(latin, [false, false, false, false, true, true, false, false, false]);
        let cyrillic = v.mask_of(Alphabet::Cyrillic);
        assert_eq!(cyrillic, [false, true, true, true, false, false, false, false, false]);
    }
}

//! TDT greedy decoding (ported from parakeet-rs): at each encoder frame the decoder/joint gives token logits and
//! duration logits; a token is emitted unless it is the blank, and the duration says how many frames to move on. The
//! decoder's state changes only when a token is emitted. A mask forbids the tokens of an alphabet that leaked.
use crate::EngineError;

/// At most this many tokens on one frame before moving on, as NeMo does: a stuck joint never loops.
pub const MAX_TOKENS_PER_FRAME: usize = 10;

/// The decoder/joint, one step at a time.
pub trait Joint {
    type State: Clone;
    /// The decoder's state before any token.
    fn initial(&self) -> Self::State;
    /// For encoder frame `frame` after token `last`: the logits (tokens with the blank, then the durations 0, 1, 2…)
    /// and the decoder's state as it would be were a token emitted now.
    fn step(&mut self, frame: usize, last: usize, state: &Self::State) -> Result<(Vec<f32>, Self::State), EngineError>;
}

/// The tokens of `frames` encoder frames. `vocab` — token logits with the blank (the rest are durations); `mask` —
/// ids that may not be emitted (the blank never is).
pub fn decode<J: Joint>(
    joint: &mut J,
    frames: usize,
    vocab: usize,
    blank: usize,
    mask: Option<&[bool]>,
) -> Result<Vec<usize>, EngineError> {
    let mut tokens = Vec::new();
    let mut state = joint.initial();
    let mut last = blank;
    let (mut t, mut on_frame) = (0, 0);
    while t < frames {
        let (logits, next) = joint.step(t, last, &state)?;
        let (token, duration) = pick(&logits, vocab, blank, mask);
        if token != blank {
            tokens.push(token);
            last = token;
            state = next;
            on_frame += 1;
        }
        if duration > 0 {
            t += duration;
            on_frame = 0;
        } else if token == blank || on_frame >= MAX_TOKENS_PER_FRAME {
            t += 1;
            on_frame = 0;
        }
    }
    Ok(tokens)
}

/// The best allowed token and the best duration.
fn pick(logits: &[f32], vocab: usize, blank: usize, mask: Option<&[bool]>) -> (usize, usize) {
    let (tokens, durations) = logits.split_at(vocab.min(logits.len()));
    let allowed = |id: usize| id == blank || mask.is_none_or(|m| !m.get(id).copied().unwrap_or(false));
    let token = tokens
        .iter()
        .enumerate()
        .filter(|(id, _)| allowed(*id))
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(blank, |(id, _)| id);
    let duration = durations.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(d, _)| d);
    (token, duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A joint that answers from a script: per frame, a list of (token, duration) it gives in order; it counts the
    /// emitted tokens it was told about through `last` to check the state follows emissions only.
    struct Scripted {
        script: Vec<Vec<(usize, usize)>>,
        cursor: Vec<usize>,
        vocab: usize,
        /// A token always one logit below the scripted one: what the mask falls back to.
        runner_up: Option<usize>,
        calls: usize,
    }

    impl Scripted {
        fn new(vocab: usize, script: Vec<Vec<(usize, usize)>>) -> Self {
            let cursor = vec![0; script.len()];
            Self { script, cursor, vocab, runner_up: None, calls: 0 }
        }
    }

    impl Joint for Scripted {
        type State = usize; // tokens emitted so far
        fn initial(&self) -> usize {
            0
        }
        fn step(&mut self, frame: usize, _last: usize, state: &usize) -> Result<(Vec<f32>, usize), EngineError> {
            self.calls += 1;
            let steps = &self.script[frame];
            let i = self.cursor[frame].min(steps.len() - 1);
            self.cursor[frame] += 1;
            let (token, duration) = steps[i];
            let mut logits = vec![0.0f32; self.vocab + 5];
            logits[token] = 10.0;
            if let Some(r) = self.runner_up {
                logits[r] = 9.0;
            }
            logits[self.vocab + duration] = 10.0;
            Ok((logits, state + 1))
        }
    }

    const BLANK: usize = 4;

    #[test]
    fn tokens_and_durations_move_through_the_frames() {
        // Frame 0: token 1 staying (duration 0), then token 2 moving 2 frames; frame 2: blank, duration 1.
        let mut j = Scripted::new(5, vec![vec![(1, 0), (2, 2)], vec![(3, 1)], vec![(BLANK, 1)]]);
        assert_eq!(decode(&mut j, 3, 5, BLANK, None).unwrap(), [1, 2], "frame 1 is skipped by the duration");
    }

    #[test]
    fn a_blank_with_duration_zero_still_moves_on() {
        let mut j = Scripted::new(5, vec![vec![(BLANK, 0)], vec![(3, 1)]]);
        assert_eq!(decode(&mut j, 2, 5, BLANK, None).unwrap(), [3]);
    }

    #[test]
    fn a_frame_gives_at_most_ten_tokens() {
        let mut j = Scripted::new(5, vec![vec![(1, 0)], vec![(BLANK, 1)]]);
        let tokens = decode(&mut j, 2, 5, BLANK, None).unwrap();
        assert_eq!(tokens, vec![1; MAX_TOKENS_PER_FRAME]);
    }

    #[test]
    fn a_masked_token_gives_way_to_the_next_best() {
        let mut j = Scripted::new(5, vec![vec![(1, 1)]]);
        j.runner_up = Some(2);
        let mask = [false, true, false, false, false];
        assert_eq!(decode(&mut j, 1, 5, BLANK, Some(&mask)).unwrap(), [2]);
    }

    #[test]
    fn the_blank_is_never_masked() {
        let mut j = Scripted::new(5, vec![vec![(BLANK, 1)]]);
        let mask = [true; 5];
        assert!(decode(&mut j, 1, 5, BLANK, Some(&mask)).unwrap().is_empty());
        assert_eq!(j.calls, 1);
    }
}

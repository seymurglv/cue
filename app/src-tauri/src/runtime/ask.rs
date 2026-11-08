//! "Ask" in the app (spec 2 §6–7): what a question was asked with, the conversation of the open answer, and the answer as
//! the windows draw it. The flow decides when to ask, paste and close; this module keeps what those effects need and turns
//! the stream into the view. Nothing of the question, the answer, the selection or the sources goes to the log.
use cue_core::ask::{
    self, AnswerPlace, AnswerView, AskContext, Segment, SelectionKind, Sources, ThinkStep, TurnState, TurnView, WriteTarget,
};
use cue_core::flow::{AskError, DictationId, PasteHow};
use cue_engines::ask::{AskEnd, AskResult, Step, assistant_turn, user_turn};
use cue_platform_win::focus::WindowRef;
use cue_platform_win::placement::PxRect;
use serde_json::Value;

/// What a question was asked with (spec 2 §6.1), taken at the chord and at the end of its recording. `Debug` shows lengths
/// only (`AskContext`).
#[derive(Clone, Debug, Default)]
pub struct Taken {
    pub ctx: AskContext,
    pub sel: SelectionKind,
    /// The selection was longer than `MAX_SELECTION_CHARS` and reached the model as its start and its end: it is never
    /// replaced (`ask::write_target`).
    pub clipped: bool,
    /// The selection's lines on screen, physical px: where the card points (none — the tray).
    pub rects: Vec<PxRect>,
    pub terminal: bool,
    /// The window the question was asked in, read at the chord: the only one an "Ask" paste goes to (ruling 6b). `None` —
    /// not known, and then nothing is pasted.
    pub window: Option<WindowRef>,
}

impl Taken {
    /// The Ctrl+C copy came: it is the selection, one that can be changed (spec 2 §6.1).
    pub fn copied(&mut self, text: &str) {
        self.ctx = with_copied(&self.ctx, text);
        self.sel = SelectionKind::Writable;
        self.clipped = text.chars().count() > ask::MAX_SELECTION_CHARS;
    }
}

/// The context once the Ctrl+C copy came (spec 2 §6.1): the copied text is the selection. The request task and the
/// conversation build the first message from it alike.
pub fn with_copied(ctx: &AskContext, text: &str) -> AskContext {
    AskContext { selection: Some(ask::clip_selection(text, ask::MAX_SELECTION_CHARS)), ..ctx.clone() }
}

/// The user message of a first question (spec 2 §6.1): the conversation and the request task build it alike.
pub fn first_turn(question: &str, ctx: &AskContext) -> Value {
    user_turn(&ask::first_message(question, ctx), None)
}

/// What the first question of a dictation was asked with, kept until the next chord: "restore" after Esc asks the same
/// question with the same context (spec 2 §4: as in dictation), though the first ask used up the focus query and the copy.
#[derive(Debug, Default)]
pub struct KeptTaken(Option<(DictationId, Taken)>);

impl KeptTaken {
    pub fn get(&self, id: DictationId) -> Option<Taken> {
        self.0.as_ref().filter(|(i, _)| *i == id).map(|(_, t)| t.clone())
    }

    pub fn keep(&mut self, id: DictationId, taken: &Taken) {
        self.0 = Some((id, taken.clone()));
    }

    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// The Ctrl+C copy of dictation `id` came after Esc dropped the request waiting for it: "restore" asks with it.
    pub fn copied(&mut self, id: DictationId, text: &str) {
        if let Some((i, taken)) = self.0.as_mut()
            && *i == id
        {
            taken.copied(text);
        }
    }

    /// The question of dictation `id` has ended for good: what it was asked with is not kept any more.
    pub fn forget(&mut self, id: DictationId) {
        self.0.take_if(|(i, _)| *i == id);
    }
}

/// What the flow hears when a request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    Answered,
    Write(PasteHow),
    Failed(AskError),
}

impl Heard {
    /// The answer shows this end (and opens, if nothing showed before): all but a write that is pasted — the answer
    /// closes with it — and a key problem, which the Drop shows itself (spec 2 §8).
    pub fn shows(self) -> bool {
        !matches!(self, Heard::Write(_) | Heard::Failed(AskError::Key))
    }
}

/// The conversation of one open answer: from the first question until `CloseAnswer` (spec 2 §6.2: forgotten after).
pub struct Conversation {
    pub taken: Taken,
    /// The answered exchanges, without cache marks (`cue_engines::ask::request_body` puts them on).
    messages: Vec<Value>,
    /// The user message of the question in flight, or of the last one that failed (Retry sends it again).
    pending: Option<Value>,
    /// The failed question a new one took the place of, its turn and its message, while the new one is in flight: an
    /// abort of the new one brings it back with its Retry (the flow's `AskAgain` names it then).
    replaced: Option<(TurnView, Value)>,
    pub view: AnswerView,
    /// The question in flight as it was sent, with the basics done (`cue_core::basics`); the turn keeps it as heard.
    sent: Option<String>,
    sources: Sources,
    /// The answer is on screen: the first step with something to show came.
    pub opened: bool,
    /// A write_to_field text waiting for `AskPaste`, already in the dictation's form.
    write: Option<String>,
    /// The turn whose text is a write shown instead of written: already in the field's form, it is pasted and copied as
    /// it is, not as an answer's Markdown (which would take off the markup the selection had).
    shown_write: Option<usize>,
    /// A write_to_field call began in the last turn's stream. One cut at `max_tokens` is asked again with more room, and
    /// that response's steps come from its start (`cue_engines::ask::run`): its first block starts the turn's words over.
    wrote: bool,
    /// A search came after the last turn's latest words: its next text block goes on a paragraph of its own, or the two
    /// would run together.
    searched: bool,
    /// The last turn has started a search: the words before its first one are gone.
    turn_searched: bool,
}

impl Conversation {
    pub fn new(taken: Taken) -> Self {
        Self {
            taken,
            messages: Vec::new(),
            pending: None,
            replaced: None,
            view: AnswerView {
                place: AnswerPlace::Tray,
                quote: None,
                turns: Vec::new(),
                sources: Vec::new(),
                listening: false,
                missed: false,
                actions: Vec::new(),
            },
            sent: None,
            sources: Sources::default(),
            opened: false,
            write: None,
            shown_write: None,
            wrote: false,
            searched: false,
            turn_searched: false,
        }
    }

    /// A question goes out. Until an exchange has succeeded it is a first question, with the context (a question asked
    /// again after a failure or a refusal replaces it); after that a follow-up, alone (spec 2 §6.2).
    pub fn ask(&mut self, question: &str) {
        self.ask_as(question, question);
    }

    /// A question that is shown (and kept in the history) as `question`, the words as heard, and goes out as `sent`.
    pub fn ask_as(&mut self, question: &str, sent: &str) {
        self.sent = Some(sent.to_owned());
        let text = if self.messages.is_empty() {
            ask::first_message(sent, &self.taken.ctx)
        } else {
            ask::follow_up_message(sent)
        };
        let previous = self.messages.last().and_then(|m| m["content"].as_array()).map(Vec::as_slice);
        let message = user_turn(&text, previous);
        let failed = self.view.turns.pop_if(|t| t.state == TurnState::Failed);
        self.replaced = failed.zip(self.pending.take());
        self.pending = Some(message);
        self.view.turns.push(TurnView::new(question));
        self.write = None;
        self.wrote = false;
        self.turn_searched = false;
        // A refused turn took the buttons away; a new question brings them back.
        self.buttons();
    }

    /// The question in flight (or the last one asked).
    pub fn question(&self) -> Option<&str> {
        self.view.turns.last().map(|t| t.question.as_str())
    }

    /// The question in flight (or the last one asked) as it goes out to the model.
    pub fn sent_question(&self) -> Option<&str> {
        self.sent.as_deref().or_else(|| self.question())
    }

    /// The Ctrl+C copy of the first question came: it is the selection, one that can be changed (spec 2 §6.1), and the
    /// question waiting to go is asked with it.
    pub fn copied(&mut self, text: &str) {
        self.taken.copied(text);
        if self.messages.is_empty()
            && let Some(question) = self.sent_question().map(str::to_owned)
        {
            self.pending = Some(first_turn(&question, &self.taken.ctx));
        }
        self.buttons();
    }

    /// Retry after a failure: the same question again, its turn starts over.
    pub fn again(&mut self) {
        if let Some(turn) = self.view.turns.last_mut() {
            *turn = TurnView::new(&turn.question);
        }
        self.wrote = false;
        self.turn_searched = false;
    }

    /// The request in flight was dropped (Esc on a follow-up): its question goes, and the answer it followed is the last
    /// turn again — not a turn that keeps "streaming" with its buttons off. A failed question it took the place of comes
    /// back, with its Retry.
    pub fn abort(&mut self) {
        let in_flight = self.view.turns.last().is_some_and(|t| matches!(t.state, TurnState::Pending | TurnState::Streaming));
        if self.pending.take().is_none() || !in_flight {
            return;
        }
        self.view.turns.pop();
        if let Some((turn, message)) = self.replaced.take() {
            self.view.turns.push(turn);
            self.pending = Some(message);
        }
        // The turn that is the last again keeps its own buttons: none under a refusal.
        self.buttons();
    }

    /// The messages of the request: the answered ones, then the question in flight.
    pub fn request(&self) -> Vec<Value> {
        self.messages.iter().cloned().chain(self.pending.clone()).collect()
    }

    /// The buttons of the open answer, for its last turn: none under a refusal (nothing to paste, spec 2 §8), the
    /// selection's otherwise (spec 2 §7). Every change of the last turn or of the selection sets them again here; before
    /// the answer opens there are none.
    fn buttons(&mut self) {
        if !self.opened {
            return;
        }
        let refused = self.view.turns.last().is_some_and(|t| t.state == TurnState::Refused);
        let t = &self.taken;
        self.view.actions = if refused { Vec::new() } else { ask::answer_actions(t.sel, t.terminal, t.clipped) };
    }

    /// The answer opens where it is shown, with the buttons of its last turn. A refusal that came before anything showed
    /// opens the answer after `end` (decision 18), still without buttons.
    pub fn open(&mut self, place: AnswerPlace) {
        self.opened = true;
        self.view.place = place;
        self.view.quote = match (place, &self.taken.ctx.selection) {
            (AnswerPlace::Card, Some(s)) => Some(ask::quote_of(s)),
            _ => None,
        };
        self.buttons();
    }

    /// A step of the stream, on the last turn. `true` — the answer has something to show now (a search, its sources, the
    /// first word): the flow opens it then (spec 2 §7: the thinking line names only real steps).
    pub fn step(&mut self, step: Step) -> bool {
        let Some(turn) = self.view.turns.last_mut() else { return false };
        // The words of a cut write's response never reach the conversation: the retry's start in its place.
        if matches!(step, Step::TextStart | Step::WriteStart) && std::mem::take(&mut self.wrote) {
            turn.segments.clear();
            turn.state = TurnState::Pending;
            self.turn_searched = false;
        }
        match step {
            Step::Searching => {
                // Words before the turn's first search only say that it is going to search, which the instruction asks
                // the model not to write: the search line takes their place.
                if !std::mem::replace(&mut self.turn_searched, true) {
                    turn.segments.clear();
                    turn.state = TurnState::Pending;
                }
                turn.step = Some(ThinkStep::Searching);
                self.searched = true;
                true
            }
            Step::Reading => {
                turn.step = Some(ThinkStep::Reading);
                self.searched = true;
                true
            }
            Step::TextStart => {
                // Words before a search and words after it are two text blocks with nothing between them: the later one
                // starts a paragraph.
                let after_search = std::mem::take(&mut self.searched) && !turn.text().trim().is_empty();
                turn.segments.push(Segment { after_search, ..Segment::default() });
                if turn.step.is_some() && turn.text().trim().is_empty() {
                    turn.step = Some(ThinkStep::Writing);
                }
                false
            }
            Step::Text(t) => {
                if turn.segments.is_empty() {
                    turn.segments.push(Segment::default());
                }
                let shows = !turn.text().trim().is_empty() || !t.trim().is_empty();
                if let Some(seg) = turn.segments.last_mut() {
                    seg.text.push_str(&t);
                }
                if shows {
                    turn.step = None;
                    turn.state = TurnState::Streaming;
                }
                shows
            }
            Step::Cite { url, title } => {
                let n = self.sources.number(&url, &title);
                if let Some(seg) = turn.segments.last_mut()
                    && !seg.cites.contains(&n)
                {
                    seg.cites.push(n);
                }
                self.view.sources = self.sources.views();
                false
            }
            Step::WriteStart => {
                self.wrote = true;
                false
            }
            Step::Spent(_) => false,
        }
    }

    /// The request ended: the last turn shows how, the exchange joins the conversation (a refusal and an empty one never
    /// do), and the flow hears what to do next (spec 2 §6.2). A failure keeps the question for Retry. `show_write` — a
    /// follow-up is under way, as the flow counts it (recording, being transcribed, held until this answer ends, or missed
    /// with its Retry): a write is shown, not pasted under the user's hands (decision 12). `may_paste(how, text)` — the
    /// window the question was asked in is still in front and may take the write as `how` would paste it: otherwise the
    /// write is shown too (ruling 6b, for a paste as for the Ctrl+C fallback).
    pub fn end(&mut self, result: &AskResult, show_write: bool, may_paste: impl FnOnce(PasteHow, &str) -> bool) -> Heard {
        self.replaced = None;
        let heard = match &result.end {
            AskEnd::Failed(why) => {
                // The words already shown stay, with the error and Retry under them (Retry starts the turn over).
                self.finish_turn(TurnState::Failed, false);
                return Heard::Failed(if why.is_key() { AskError::Key } else { AskError::Failed });
            }
            AskEnd::Refused => {
                // The partial words are discarded and the exchange is forgotten; nothing to paste (spec 2 §8).
                self.finish_turn(TurnState::Refused, true);
                self.buttons();
                self.pending = None;
                return Heard::Answered;
            }
            AskEnd::Answer => {
                self.finish_turn(TurnState::Done, false);
                Heard::Answered
            }
            AskEnd::Write { text, replace } => {
                let text = ask::field_text(self.question().unwrap_or(""), &self.taken.ctx, text);
                let t = &self.taken;
                let target = match ask::write_target(*replace, t.sel, t.terminal, t.clipped) {
                    _ if show_write => WriteTarget::Show,
                    WriteTarget::Paste(how) if !may_paste(how, &text) => {
                        tracing::info!("ask: the window in front is not the one asked in, or is a terminal: the write is shown");
                        WriteTarget::Show
                    }
                    target => target,
                };
                // The turn ends with the text either way: a write that is pasted closes the answer, but should it stay
                // on screen (a follow-up under way), the turn still shows what was written.
                if let Some(turn) = self.view.turns.last_mut() {
                    turn.segments = vec![Segment { text: text.clone(), ..Segment::default() }];
                }
                self.finish_turn(TurnState::Done, false);
                match target {
                    WriteTarget::Paste(how) => {
                        self.write = Some(text);
                        Heard::Write(how)
                    }
                    // Shown instead of written: an answer to copy, as it was written.
                    WriteTarget::Show => {
                        self.shown_write = self.view.turns.len().checked_sub(1);
                        Heard::Answered
                    }
                }
            }
        };
        let turn = assistant_turn(&result.content);
        if turn["content"].as_array().is_some_and(|c| !c.is_empty()) {
            if let Some(question) = self.pending.take() {
                self.messages.push(question);
            }
            self.messages.push(turn);
        } else {
            self.pending = None;
        }
        heard
    }

    fn finish_turn(&mut self, state: TurnState, clear: bool) {
        if let Some(turn) = self.view.turns.last_mut() {
            turn.state = state;
            turn.step = None;
            if clear {
                turn.segments.clear();
            }
        }
    }

    /// The model wrote text for the field (the paste that follows is its attempt's result).
    pub fn has_write(&self) -> bool {
        self.write.is_some()
    }

    /// What `AskPaste` and Copy put in: a write's text, or the last answer's plain text.
    pub fn paste_text(&self) -> Option<String> {
        if let Some(w) = &self.write {
            return Some(w.clone());
        }
        let turn = self.view.turns.last().filter(|t| t.state == TurnState::Done)?;
        let shown_write = self.shown_write.is_some_and(|i| i + 1 == self.view.turns.len());
        let text = if shown_write { turn.text() } else { ask::plain_answer(&turn.text()) };
        (!text.trim().is_empty()).then_some(text)
    }

    /// The page a click on source `n` opens: only a web page (spec 2 §7).
    pub fn source_url(&self, n: u32) -> Option<&str> {
        self.sources.url(n).filter(|u| ask::openable(u))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cue_core::ask::{AnswerAction, SourceView};
    use cue_engines::ask::{AskFailure, AskUsage};
    use serde_json::json;

    fn taken(sel: SelectionKind, terminal: bool) -> Taken {
        Taken {
            ctx: AskContext {
                app: "Telegram Desktop".into(),
                before_cursor: None,
                selection: (sel != SelectionKind::None).then(|| "Can you send the report by Friday?".to_owned()),
                today: "2026-10-28".into(),
            },
            sel,
            clipped: false,
            rects: Vec::new(),
            terminal,
            window: Some(WindowRef { window: 0x10, process: 100, app_process: None }),
        }
    }

    /// The window the question was asked in is in front and takes any paste.
    fn anywhere(_how: PasteHow, _text: &str) -> bool {
        true
    }

    fn result(end: AskEnd, content: Vec<Value>) -> AskResult {
        AskResult { end, content, usage: AskUsage::default() }
    }

    /// Ruling 6b, for a write: the window the question was asked in is not in front any more (or is a terminal for a
    /// text with line breaks), so the write is shown in the answer, with Copy, and the conversation records it as shown.
    #[test]
    fn a_write_whose_window_is_gone_is_shown_with_copy() {
        let call = json!({ "type": "tool_use", "id": "toolu_1", "name": "write_to_field", "input": { "text": "x", "mode": "insert" } });
        let mut c = Conversation::new(taken(SelectionKind::Writable, false));
        c.ask("ответь");
        let mut asked = None;
        let heard = c.end(&result(AskEnd::Write { text: "Пришлю в четверг.".into(), replace: false }, vec![call]), false, |how, text| {
            asked = Some((how, text.to_owned()));
            false
        });
        assert_eq!(heard, Heard::Answered, "shown instead of written");
        assert_eq!(asked, Some((PasteHow::Below, "Пришлю в четверг.".to_owned())), "asked with where and what it would paste");
        assert!(!c.has_write());
        c.open(AnswerPlace::Tray);
        assert_eq!((c.view.turns[0].text(), c.paste_text().as_deref()), ("Пришлю в четверг.".to_owned(), Some("Пришлю в четверг.")));
        c.ask("а короче");
        assert_eq!(c.request()[2]["content"][0]["type"], "tool_result", "the follow-up says the text was shown");
        // A write that is shown anyway (a follow-up under way) never asks.
        let mut d = Conversation::new(taken(SelectionKind::None, false));
        d.ask("напиши");
        let shown = d.end(&result(AskEnd::Write { text: "x".into(), replace: false }, vec![]), true, |_, _| panic!("not asked"));
        assert_eq!(shown, Heard::Answered);
    }

    /// A selection that reached the model clipped is never replaced: the write is shown, and there is no Replace button.
    #[test]
    fn a_clipped_selection_is_never_replaced() {
        let mut c = Conversation::new(Taken { clipped: true, ..taken(SelectionKind::Writable, false) });
        c.ask("исправь опечатки");
        assert_eq!(c.end(&result(AskEnd::Write { text: "Fixed.".into(), replace: true }, vec![]), false, anywhere), Heard::Answered);
        c.open(AnswerPlace::Card);
        assert_eq!(c.view.actions, [AnswerAction::Below, AnswerAction::Copy]);
        let mut t = Taken::default();
        t.copied(&"а".repeat(ask::MAX_SELECTION_CHARS + 1));
        assert!(t.clipped && t.ctx.selection.as_deref().is_some_and(|s| s.contains(ask::ELISION)), "a long Ctrl+C copy is clipped too");
        t.copied("коротко");
        assert!(!t.clipped);
    }

    /// Audit M1: a failure after words were shown keeps them, with the error and Retry under them; Retry starts over.
    #[test]
    fn a_failure_keeps_the_words_already_shown() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("объясни подробно");
        c.step(Step::TextStart);
        c.step(Step::Text("Начало ответа".into()));
        assert_eq!(c.end(&result(AskEnd::Failed(AskFailure::Timeout), vec![]), false, anywhere), Heard::Failed(AskError::Failed));
        assert_eq!((c.view.turns[0].state, c.view.turns[0].text()), (TurnState::Failed, "Начало ответа".to_owned()));
        assert_eq!(c.paste_text(), None, "a cut answer is not pasted");
        c.again();
        assert!(c.view.turns[0].segments.is_empty(), "Retry starts the words over");
    }

    /// The selection counts as said (C1): a replace over Markdown keeps its markup, end to end.
    #[test]
    fn a_replace_over_markdown_keeps_its_markup() {
        let readme = "## Install\n- one;\n- two;";
        let mut c = Conversation::new(Taken {
            ctx: AskContext { selection: Some(readme.into()), ..taken(SelectionKind::Writable, false).ctx },
            ..taken(SelectionKind::Writable, false)
        });
        c.ask("исправь");
        let fixed = "## Installation\n- one;\n- two;";
        assert_eq!(c.end(&result(AskEnd::Write { text: fixed.into(), replace: true }, vec![]), false, anywhere), Heard::Write(PasteHow::Replace));
        assert_eq!(c.paste_text().as_deref(), Some(fixed));

        // Shown instead (the window moved on): Copy and Insert give the text as written, not as an answer's Markdown.
        let mut s = Conversation::new(Taken {
            ctx: AskContext { selection: Some(readme.into()), ..taken(SelectionKind::Writable, false).ctx },
            ..taken(SelectionKind::Writable, false)
        });
        s.ask("исправь");
        assert_eq!(s.end(&result(AskEnd::Write { text: fixed.into(), replace: true }, vec![]), false, |_, _| false), Heard::Answered);
        assert_eq!(s.paste_text().as_deref(), Some(fixed));
        s.ask("а теперь объясни");
        s.step(Step::TextStart);
        s.step(Step::Text("**Важно:** заголовок".into()));
        s.end(&result(AskEnd::Answer, vec![text_block("x")]), false, anywhere);
        assert_eq!(s.paste_text().as_deref(), Some("Важно: заголовок"), "a later answer is pasted as an answer again");
    }

    fn text_block(t: &str) -> Value {
        json!({ "type": "text", "text": t })
    }

    fn first_text(c: &Conversation) -> String {
        c.request()[0]["content"][0]["text"].as_str().unwrap().to_owned()
    }

    #[test]
    fn the_first_question_carries_the_context_and_a_follow_up_does_not() {
        let mut c = Conversation::new(taken(SelectionKind::Writable, false));
        c.ask("ответь что пришлю в четверг");
        assert_eq!(c.question(), Some("ответь что пришлю в четверг"));
        let first = c.request();
        assert_eq!(first.len(), 1);
        assert!(first_text(&c).starts_with("<question>ответь что пришлю в четверг</question>\n<selection>Can you send"));
        c.step(Step::TextStart);
        c.step(Step::Text("Пришлю в четверг.".into()));
        assert_eq!(c.end(&result(AskEnd::Answer, vec![text_block("Пришлю в четверг.")]), false, anywhere), Heard::Answered);
        c.ask("а короче");
        let next = c.request();
        assert_eq!(next.len(), 3, "the question, the answer, the follow-up");
        assert_eq!(next[1]["role"], "assistant");
        assert_eq!(next[2]["content"][0]["text"], "<question>а короче</question>");
        assert_eq!(c.view.turns.len(), 2);
    }

    #[test]
    fn steps_build_the_last_turn_and_say_when_there_is_something_to_show() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("какая последняя версия Rust");
        assert!(c.step(Step::Searching), "a search shows");
        assert_eq!(c.view.turns[0].step, Some(ThinkStep::Searching));
        assert!(c.step(Step::Reading));
        assert!(!c.step(Step::TextStart));
        assert_eq!(c.view.turns[0].step, Some(ThinkStep::Writing), "after the sources, the words are being written");
        assert!(!c.step(Step::Text(" ".into())), "a space is not a word yet");
        assert!(c.step(Step::Text("Rust 1.95".into())));
        assert_eq!(c.view.turns[0].step, None);
        assert_eq!(c.view.turns[0].state, TurnState::Streaming);
        assert!(!c.step(Step::Cite { url: "https://blog.rust-lang.org/".into(), title: "Rust Blog".into() }));
        assert!(!c.step(Step::Cite { url: "https://blog.rust-lang.org/".into(), title: "Rust Blog".into() }));
        assert_eq!(c.view.turns[0].segments[0].cites, [1], "one number per source in a segment");
        assert_eq!(c.view.sources, [SourceView { n: 1, site: "blog.rust-lang.org".into(), title: "Rust Blog".into() }]);
        assert!(!c.step(Step::WriteStart), "a write never opens the answer");
        assert!(!c.step(Step::Spent(AskUsage::default())), "the cost is the runtime's, not the view's");
    }

    /// Words before the turn's first search only say that it is going to search: the search line takes their place, and
    /// they are neither shown nor pasted ("Это вопрос о текущей погоде, поэтому нужен поиск." before).
    #[test]
    fn words_before_the_first_search_go() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("какая погода в Баку");
        c.open(AnswerPlace::Tray);
        c.step(Step::TextStart);
        assert!(c.step(Step::Text("Погода меняется, поэтому проверю актуальные данные.".into())));
        assert!(c.step(Step::Searching));
        let turn = &c.view.turns[0];
        assert_eq!((turn.segments.len(), turn.state, turn.step), (0, TurnState::Pending, Some(ThinkStep::Searching)));
        c.step(Step::Reading);
        c.step(Step::TextStart);
        c.step(Step::Text("В Баку +18 °C.".into()));
        assert!(!c.view.turns[0].segments[0].after_search, "no words before it are left to set apart");
        c.end(&result(AskEnd::Answer, vec![text_block("x")]), false, anywhere);
        assert_eq!(c.paste_text().as_deref(), Some("В Баку +18 °C."));
    }

    /// Words between two searches and words after the second one are two text blocks with nothing between them: the
    /// later one starts a paragraph, on screen and in a paste ("…данные.Поиск…" before). A citation splits a block too,
    /// with no search between.
    #[test]
    fn words_after_a_later_search_start_a_paragraph() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("какая погода в Баку");
        c.step(Step::Searching);
        c.step(Step::Reading);
        c.step(Step::TextStart);
        c.step(Step::Text("Сейчас в Баку ".into()));
        c.step(Step::TextStart);
        c.step(Step::Text("+18 °C.".into()));
        c.step(Step::Cite { url: "https://a.io/x".into(), title: "A".into() });
        c.step(Step::Searching);
        c.step(Step::Reading);
        c.step(Step::TextStart);
        c.step(Step::Text("Ветер слабый.".into()));
        let flags: Vec<bool> = c.view.turns[0].segments.iter().map(|s| s.after_search).collect();
        assert_eq!(flags, [false, false, true], "only the block after the later search");
        c.end(&result(AskEnd::Answer, vec![text_block("x")]), false, anywhere);
        assert_eq!(c.paste_text().as_deref(), Some("Сейчас в Баку +18 °C.\n\nВетер слабый."));

        // A follow-up's own first search drops its own words before it, not the answer it follows.
        c.ask("а завтра");
        c.step(Step::TextStart);
        c.step(Step::Text("Проверю прогноз.".into()));
        c.step(Step::Searching);
        assert_eq!((c.view.turns[0].text(), c.view.turns[1].segments.len()), ("Сейчас в Баку +18 °C.\n\nВетер слабый.".to_owned(), 0));
    }

    /// A write cut at `max_tokens` is asked again with more room, and its steps come again from the start: the turn's
    /// words start over with them, they are not added to the cut response's.
    #[test]
    fn a_write_asked_again_starts_its_words_over() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("напиши письмо Ане");
        c.step(Step::TextStart);
        assert!(c.step(Step::Text("Сейчас напишу.".into())));
        c.step(Step::Cite { url: "https://a.io/x".into(), title: "A".into() });
        assert!(!c.step(Step::WriteStart));
        // The retry answers this time.
        assert!(!c.step(Step::TextStart));
        assert_eq!(
            (c.view.turns[0].text(), c.view.turns[0].state, c.view.turns[0].segments[0].cites.len()),
            (String::new(), TurnState::Pending, 0),
            "the cut response's words and marks go"
        );
        assert!(c.step(Step::Text("Письмо готово.".into())));
        c.step(Step::TextStart);
        c.step(Step::Text(" Отправить?".into()));
        assert_eq!(c.view.turns[0].text(), "Письмо готово. Отправить?", "a second block of the retry adds to it");
        assert_eq!(c.end(&result(AskEnd::Answer, vec![text_block("Письмо готово. Отправить?")]), false, anywhere), Heard::Answered);
        assert_eq!(c.paste_text().as_deref(), Some("Письмо готово. Отправить?"));

        // The retry writes again: its call starts the words over too, after a search of its own.
        let mut w = Conversation::new(taken(SelectionKind::None, false));
        w.ask("напиши письмо Ане");
        w.step(Step::TextStart);
        w.step(Step::Text("Пишу.".into()));
        w.step(Step::WriteStart);
        assert!(w.step(Step::Searching));
        assert!(!w.step(Step::WriteStart));
        assert_eq!((w.view.turns[0].segments.len(), w.view.turns[0].state), (0, TurnState::Pending));
        assert_eq!(w.end(&result(AskEnd::Write { text: "Привет, Аня!".into(), replace: false }, vec![]), false, anywhere), Heard::Write(PasteHow::AtCaret));
        assert_eq!(w.paste_text().as_deref(), Some("Привет, Аня!"));
    }

    #[test]
    fn a_failed_question_is_sent_again_and_a_new_one_replaces_it() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("курс евро");
        c.step(Step::Searching);
        assert_eq!(c.end(&result(AskEnd::Failed(AskFailure::Timeout), vec![]), false, anywhere), Heard::Failed(AskError::Failed));
        assert_eq!(c.view.turns[0].state, TurnState::Failed);
        assert_eq!(c.request().len(), 1, "the question is kept for Retry");
        c.again();
        assert_eq!((c.view.turns.len(), c.view.turns[0].state), (1, TurnState::Pending));
        assert_eq!(c.end(&result(AskEnd::Failed(AskFailure::Key(401)), vec![]), false, anywhere), Heard::Failed(AskError::Key));
        c.ask("курс доллара");
        assert_eq!(c.view.turns.len(), 1, "the failed turn gives way to the new question");
        let text = first_text(&c);
        assert!(text.contains("<question>курс доллара</question>") && text.contains("<app>"), "still the first question: {text}");
    }

    /// A refusal shows no partial words and offers no paste; the refused exchange is forgotten — an assistant message with
    /// nothing in it is refused by the API, and the partial words must not reach the model again.
    #[test]
    fn a_refusal_shows_no_words_no_buttons_and_leaves_no_trace() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("q");
        c.open(AnswerPlace::Tray);
        c.step(Step::TextStart);
        c.step(Step::Text("Я не".into()));
        assert_eq!(c.end(&result(AskEnd::Refused, vec![]), false, anywhere), Heard::Answered);
        assert_eq!((c.view.turns[0].state, c.view.turns[0].segments.len()), (TurnState::Refused, 0));
        assert!(c.view.actions.is_empty(), "nothing to paste");
        assert_eq!(c.paste_text(), None);
        c.ask("а другое");
        assert_eq!(c.request().len(), 1, "no assistant message: a first question again");
        assert!(first_text(&c).contains("<app>"));
        assert_eq!(c.view.actions, [AnswerAction::Insert, AnswerAction::Copy], "a new question brings the buttons back");

        // Refused before anything showed: the answer opens only after the end, still without buttons.
        let mut r = Conversation::new(taken(SelectionKind::None, false));
        r.ask("q");
        r.end(&result(AskEnd::Refused, vec![]), false, anywhere);
        r.open(AnswerPlace::Tray);
        assert!(r.view.actions.is_empty(), "opened after a refusal: no buttons");

        let mut e = Conversation::new(taken(SelectionKind::None, false));
        e.ask("q");
        e.end(&result(AskEnd::Answer, vec![]), false, anywhere);
        e.ask("q2");
        assert_eq!(e.request().len(), 1, "never an assistant message with no blocks");

        // A follow-up aborted after a refusal: the refused turn is the last again, and its buttons stay off.
        let mut f = Conversation::new(taken(SelectionKind::Writable, false));
        f.ask("q");
        f.open(AnswerPlace::Tray);
        f.end(&result(AskEnd::Refused, vec![]), false, anywhere);
        f.ask("а другое");
        assert_eq!(f.view.actions, [AnswerAction::Below, AnswerAction::Replace, AnswerAction::Copy]);
        f.abort();
        assert_eq!((f.view.turns.len(), f.view.turns[0].state), (1, TurnState::Refused));
        assert!(f.view.actions.is_empty(), "no paste of a refused answer");
    }

    #[test]
    fn writes_go_where_the_selection_says_in_the_dictation_form() {
        let write = |text: &str, replace: bool| result(AskEnd::Write { text: text.into(), replace }, vec![]);
        let mut c = Conversation::new(taken(SelectionKind::Writable, false));
        c.ask("ответь");
        assert_eq!(c.end(&write("**Пришлю** в четверг.", false), false, anywhere), Heard::Write(PasteHow::Below));
        assert!(c.has_write());
        assert_eq!(c.paste_text().as_deref(), Some("Пришлю в четверг."), "markup the question did not ask for goes");
        assert_eq!(
            (c.view.turns[0].text(), c.view.turns[0].state),
            ("Пришлю в четверг.".to_owned(), TurnState::Done),
            "the turn ends with the text it wrote, should it stay on screen"
        );

        // Decisions 3–4: a write about read-only text is shown in the answer, with Copy.
        let mut c = Conversation::new(taken(SelectionKind::ReadOnly, false));
        c.ask("ответь");
        assert_eq!(c.end(&write("Пришлю.", false), false, anywhere), Heard::Answered);
        c.open(AnswerPlace::Tray);
        assert_eq!((c.view.turns[0].text(), c.view.actions.as_slice()), ("Пришлю.".to_owned(), &[AnswerAction::Copy][..]));
        assert!(!c.has_write());

        let mut c = Conversation::new(taken(SelectionKind::Writable, true));
        c.ask("исправь");
        assert_eq!(c.end(&write("git push origin main", true), false, anywhere), Heard::Answered, "no replace in a terminal: shown");
        assert_eq!(c.view.turns[0].text(), "git push origin main");

        // Decision 12: the request ended while a follow-up records — a write is shown, never pasted under the user's hands.
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("напиши письмо");
        assert_eq!(c.end(&write("Привет!", false), true, anywhere), Heard::Answered);
        assert_eq!(c.paste_text().as_deref(), Some("Привет!"));
    }

    /// The answer shows for every end but a write that is pasted (the answer never shows then) and a key problem (the
    /// Drop's own error instead, spec 2 §8).
    #[test]
    fn every_end_but_a_pasted_write_or_a_key_problem_shows_the_answer() {
        assert!(Heard::Answered.shows());
        assert!(Heard::Failed(AskError::Failed).shows(), "the error with Retry");
        assert!(!Heard::Failed(AskError::Key).shows());
        for how in [PasteHow::AtCaret, PasteHow::Below, PasteHow::Replace] {
            assert!(!Heard::Write(how).shows(), "{how:?}");
        }
    }

    #[test]
    fn the_paste_of_an_answer_is_its_plain_text() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("q");
        assert_eq!(c.paste_text(), None, "nothing yet");
        c.step(Step::TextStart);
        c.step(Step::Text("**Коротко:** оба.\n* первый".into()));
        c.end(&result(AskEnd::Answer, vec![text_block("x")]), false, anywhere);
        assert_eq!(c.paste_text().as_deref(), Some("Коротко: оба.\n- первый"));
    }

    /// The buttons follow the selection, in the tray too.
    #[test]
    fn the_card_quotes_the_selection_and_both_places_offer_the_selections_buttons() {
        let mut c = Conversation::new(taken(SelectionKind::Writable, false));
        c.ask("объясни");
        c.open(AnswerPlace::Card);
        assert!(c.opened);
        assert_eq!(c.view.quote.as_deref(), Some("Can you send the report by Friday?"));
        assert_eq!(c.view.actions, [AnswerAction::Below, AnswerAction::Replace, AnswerAction::Copy]);
        let mut t = Conversation::new(taken(SelectionKind::Writable, false));
        t.ask("объясни");
        t.open(AnswerPlace::Tray);
        assert_eq!(t.view.quote, None);
        assert_eq!(t.view.actions, [AnswerAction::Below, AnswerAction::Replace, AnswerAction::Copy], "never Insert over a selection");
    }

    #[test]
    fn only_web_pages_open() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("q");
        c.step(Step::TextStart);
        c.step(Step::Cite { url: "https://a.io/x".into(), title: "A".into() });
        c.step(Step::Cite { url: "file:///C:/x.exe".into(), title: "B".into() });
        assert_eq!((c.source_url(1), c.source_url(2), c.source_url(3)), (Some("https://a.io/x"), None, None));
    }

    #[test]
    fn a_shown_write_is_answered_before_the_follow_up() {
        let call = json!({ "type": "tool_use", "id": "toolu_1", "name": "write_to_field", "input": { "text": "x", "mode": "replace" } });
        let mut c = Conversation::new(taken(SelectionKind::Writable, true));
        c.ask("исправь");
        c.end(&result(AskEnd::Write { text: "x".into(), replace: true }, vec![call]), false, anywhere);
        c.ask("а иначе");
        let next = c.request();
        assert_eq!(next[2]["content"][0]["type"], "tool_result");
        assert_eq!(next[2]["content"][0]["tool_use_id"], "toolu_1");
    }

    /// Esc on a follow-up: its question and its turn go, the answer it followed is the last again.
    #[test]
    fn an_aborted_follow_up_leaves_the_answer_it_follows() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("q");
        c.end(&result(AskEnd::Answer, vec![text_block("a")]), false, anywhere);
        c.ask("а подробнее");
        c.step(Step::TextStart);
        c.abort();
        assert_eq!(c.view.turns.len(), 1, "the follow-up's turn goes");
        c.ask("а короче");
        assert_eq!(c.request().len(), 3, "the aborted question is not in the history");
    }

    /// A follow-up asked in place of a failed question and aborted before its answer showed: the failed question is the
    /// last again, with its Retry (the flow's `AskAgain` names it).
    #[test]
    fn an_aborted_follow_up_brings_back_the_failed_question_it_replaced() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("курс евро");
        c.end(&result(AskEnd::Failed(AskFailure::Timeout), vec![]), false, anywhere);
        c.ask("а доллара");
        c.step(Step::Searching);
        c.abort();
        assert_eq!(c.view.turns.len(), 1);
        assert_eq!((c.view.turns[0].question.as_str(), c.view.turns[0].state), ("курс евро", TurnState::Failed));
        c.again();
        assert_eq!(c.request().len(), 1);
        assert!(first_text(&c).starts_with("<question>курс евро</question>"), "Retry sends the failed question");

        // Once the new question has ended, the failed one is gone for good.
        let mut d = Conversation::new(taken(SelectionKind::None, false));
        d.ask("курс евро");
        d.end(&result(AskEnd::Failed(AskFailure::Timeout), vec![]), false, anywhere);
        d.ask("а доллара");
        d.end(&result(AskEnd::Answer, vec![text_block("90")]), false, anywhere);
        d.ask("а юаня");
        d.abort();
        assert_eq!(d.view.turns.iter().map(|t| t.question.as_str()).collect::<Vec<_>>(), ["а доллара"]);
    }

    /// The Ctrl+C copy came after the question was asked: the first message is rebuilt as the request task builds it.
    #[test]
    fn a_copied_selection_becomes_the_first_questions_selection() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("переведи");
        c.copied("Спасибо, посмотрю вечером");
        let text = first_text(&c);
        assert!(text.contains("<selection>Спасибо, посмотрю вечером</selection>"), "{text}");
        let ctx = with_copied(&taken(SelectionKind::None, false).ctx, "Спасибо, посмотрю вечером");
        assert_eq!(c.request()[0], first_turn("переведи", &ctx), "the same message the request task sends");
        c.open(AnswerPlace::Tray);
        assert_eq!(c.view.actions, [AnswerAction::Below, AnswerAction::Replace, AnswerAction::Copy], "a copied selection can be changed");

        let mut o = Conversation::new(taken(SelectionKind::None, false));
        o.ask("переведи");
        o.open(AnswerPlace::Tray);
        assert_eq!(o.view.actions, [AnswerAction::Insert, AnswerAction::Copy]);
        o.copied("Спасибо");
        assert_eq!(o.view.actions, [AnswerAction::Below, AnswerAction::Replace, AnswerAction::Copy], "the open answer's buttons follow");
    }

    /// The history and the answer show the question as it was heard; the model gets the one with the basics done, also
    /// when the Ctrl+C copy rebuilds the first message.
    #[test]
    fn the_question_is_shown_as_heard_and_sent_with_the_basics_done() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask_as("эм, переведи", "Переведи");
        assert_eq!(c.question(), Some("эм, переведи"));
        assert_eq!(c.sent_question(), Some("Переведи"));
        assert_eq!(c.request()[0], first_turn("Переведи", &taken(SelectionKind::None, false).ctx));
        c.copied("Спасибо");
        let ctx = with_copied(&taken(SelectionKind::None, false).ctx, "Спасибо");
        assert_eq!(c.request()[0], first_turn("Переведи", &ctx), "the copy keeps the tidied question");
        assert_eq!(c.view.turns[0].question, "эм, переведи", "the history reads this");
    }

    /// A Ctrl+C copy that comes after Esc dropped the request waiting for it: what "restore" asks with gets it all the
    /// same; once the question has ended for good, its context is forgotten.
    #[test]
    fn a_copy_that_comes_after_esc_still_reaches_restore() {
        let mut kept = KeptTaken::default();
        kept.keep(DictationId(4), &taken(SelectionKind::None, false));
        kept.copied(DictationId(5), "чужое");
        assert_eq!(kept.get(DictationId(4)).unwrap().ctx.selection, None, "another dictation's copy");
        kept.copied(DictationId(4), "Спасибо, посмотрю вечером");
        let t = kept.get(DictationId(4)).unwrap();
        assert_eq!((t.ctx.selection.as_deref(), t.sel), (Some("Спасибо, посмотрю вечером"), SelectionKind::Writable));
        kept.forget(DictationId(5));
        assert!(kept.get(DictationId(4)).is_some(), "only its own dictation's is forgotten");
        kept.forget(DictationId(4));
        assert!(kept.get(DictationId(4)).is_none());
    }

    /// "Restore" after Esc asks with the context taken for its dictation: the first ask used up the focus query.
    #[test]
    fn restore_asks_with_the_context_kept_for_its_dictation() {
        let mut kept = KeptTaken::default();
        kept.keep(DictationId(4), &taken(SelectionKind::Writable, false));
        assert!(kept.get(DictationId(5)).is_none(), "another dictation takes its own");
        let mut c = Conversation::new(kept.get(DictationId(4)).expect("kept"));
        c.ask("переведи");
        assert!(first_text(&c).contains("<selection>Can you send the report by Friday?</selection>"));
        kept.clear();
        assert!(kept.get(DictationId(4)).is_none());
    }
}

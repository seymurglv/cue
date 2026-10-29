//! "Ask" in the app (spec 2 §6–7): what a question was asked with, the conversation of the open answer, and the answer as
//! the windows draw it. The flow decides when to ask, paste and close; this module keeps what those effects need and turns
//! the stream into the view. Nothing of the question, the answer, the selection or the sources goes to the log.
use cue_core::ask::{
    self, AnswerPlace, AnswerView, AskContext, Segment, SelectionKind, Sources, ThinkStep, TurnState, TurnView, WriteTarget,
};
use cue_core::flow::{AskError, DictationId, PasteHow};
use cue_engines::ask::{AskEnd, AskResult, Step, assistant_turn, user_turn};
use cue_platform_win::placement::PxRect;
use serde_json::Value;

/// What a question was asked with (spec 2 §6.1), taken at the chord and at the end of its recording. `Debug` shows lengths
/// only (`AskContext`).
#[derive(Clone, Debug, Default)]
pub struct Taken {
    pub ctx: AskContext,
    pub sel: SelectionKind,
    /// The selection's lines on screen, physical px: where the card points (none — the tray).
    #[allow(dead_code)] // not read until the card placement uses it
    pub rects: Vec<PxRect>,
    pub terminal: bool,
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
}

/// What the flow hears when a request ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    Answered,
    Write(PasteHow),
    Failed(AskError),
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
    /// The code execution container of a filtered search, sent back with the next request.
    pub container: Option<String>,
    pub view: AnswerView,
    sources: Sources,
    /// The answer is on screen: the first step with something to show came.
    pub opened: bool,
    /// A write_to_field text waiting for `AskPaste`, already in the dictation's form.
    write: Option<String>,
    /// A write_to_field call began in the last turn's stream. One cut at `max_tokens` is asked again with more room, and
    /// that response's steps come from its start (`cue_engines::ask::run`): its first block starts the turn's words over.
    wrote: bool,
}

impl Conversation {
    pub fn new(taken: Taken) -> Self {
        Self {
            taken,
            messages: Vec::new(),
            pending: None,
            replaced: None,
            container: None,
            view: AnswerView {
                place: AnswerPlace::Tray,
                quote: None,
                turns: Vec::new(),
                sources: Vec::new(),
                listening: false,
                missed: false,
                actions: Vec::new(),
            },
            sources: Sources::default(),
            opened: false,
            write: None,
            wrote: false,
        }
    }

    /// A question goes out. Until an exchange has succeeded it is a first question, with the context (a question asked
    /// again after a failure or a refusal replaces it); after that a follow-up, alone (spec 2 §6.2).
    pub fn ask(&mut self, question: &str) {
        let text = if self.messages.is_empty() {
            ask::first_message(question, &self.taken.ctx)
        } else {
            ask::follow_up_message(question)
        };
        let previous = self.messages.last().and_then(|m| m["content"].as_array()).map(Vec::as_slice);
        let message = user_turn(&text, previous);
        let failed = self.view.turns.pop_if(|t| t.state == TurnState::Failed);
        self.replaced = failed.zip(self.pending.take());
        self.pending = Some(message);
        self.view.turns.push(TurnView::new(question));
        self.write = None;
        self.wrote = false;
        // A refused turn took the buttons away; a new question brings them back.
        self.buttons();
    }

    /// The question in flight (or the last one asked).
    pub fn question(&self) -> Option<&str> {
        self.view.turns.last().map(|t| t.question.as_str())
    }

    /// The Ctrl+C copy of the first question came: it is the selection, one that can be changed (spec 2 §6.1), and the
    /// question waiting to go is asked with it.
    pub fn copied(&mut self, text: &str) {
        self.taken.ctx = with_copied(&self.taken.ctx, text);
        self.taken.sel = SelectionKind::Writable;
        if self.messages.is_empty()
            && let Some(question) = self.question().map(str::to_owned)
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
        self.view.actions = if refused { Vec::new() } else { ask::answer_actions(self.taken.sel, self.taken.terminal) };
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
        }
        match step {
            Step::Searching => {
                turn.step = Some(ThinkStep::Searching);
                true
            }
            Step::Reading => {
                turn.step = Some(ThinkStep::Reading);
                true
            }
            Step::TextStart => {
                turn.segments.push(Segment::default());
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
    /// with its Retry): a write is shown, not pasted under the user's hands (decision 12).
    pub fn end(&mut self, result: &AskResult, show_write: bool) -> Heard {
        self.replaced = None;
        let heard = match &result.end {
            AskEnd::Failed(why) => {
                self.finish_turn(TurnState::Failed, true);
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
                let text = ask::field_text(self.question().unwrap_or(""), text);
                let target = if show_write { WriteTarget::Show } else { ask::write_target(*replace, self.taken.sel, self.taken.terminal) };
                // The turn ends with the text either way: a write that is pasted closes the answer, but should it stay
                // on screen (a follow-up under way), the turn still shows what was written.
                if let Some(turn) = self.view.turns.last_mut() {
                    turn.segments = vec![Segment { text: text.clone(), cites: Vec::new() }];
                }
                self.finish_turn(TurnState::Done, false);
                match target {
                    WriteTarget::Paste(how) => {
                        self.write = Some(text);
                        Heard::Write(how)
                    }
                    // Shown instead of written: an answer to copy.
                    WriteTarget::Show => Heard::Answered,
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
        if result.container.is_some() {
            self.container.clone_from(&result.container);
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
        let text = ask::plain_answer(&turn.text());
        (!text.is_empty()).then_some(text)
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
            rects: Vec::new(),
            terminal,
        }
    }

    fn result(end: AskEnd, content: Vec<Value>) -> AskResult {
        AskResult { end, content, usage: AskUsage::default(), container: None }
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
        assert_eq!(c.end(&result(AskEnd::Answer, vec![text_block("Пришлю в четверг.")]), false), Heard::Answered);
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
        assert_eq!(c.end(&result(AskEnd::Answer, vec![text_block("Письмо готово. Отправить?")]), false), Heard::Answered);
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
        assert_eq!(w.end(&result(AskEnd::Write { text: "Привет, Аня!".into(), replace: false }, vec![]), false), Heard::Write(PasteHow::AtCaret));
        assert_eq!(w.paste_text().as_deref(), Some("Привет, Аня!"));
    }

    #[test]
    fn a_failed_question_is_sent_again_and_a_new_one_replaces_it() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("курс евро");
        c.step(Step::Searching);
        assert_eq!(c.end(&result(AskEnd::Failed(AskFailure::Timeout), vec![]), false), Heard::Failed(AskError::Failed));
        assert_eq!(c.view.turns[0].state, TurnState::Failed);
        assert_eq!(c.request().len(), 1, "the question is kept for Retry");
        c.again();
        assert_eq!((c.view.turns.len(), c.view.turns[0].state), (1, TurnState::Pending));
        assert_eq!(c.end(&result(AskEnd::Failed(AskFailure::Key(401)), vec![]), false), Heard::Failed(AskError::Key));
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
        assert_eq!(c.end(&result(AskEnd::Refused, vec![]), false), Heard::Answered);
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
        r.end(&result(AskEnd::Refused, vec![]), false);
        r.open(AnswerPlace::Tray);
        assert!(r.view.actions.is_empty(), "opened after a refusal: no buttons");

        let mut e = Conversation::new(taken(SelectionKind::None, false));
        e.ask("q");
        e.end(&result(AskEnd::Answer, vec![]), false);
        e.ask("q2");
        assert_eq!(e.request().len(), 1, "never an assistant message with no blocks");

        // A follow-up aborted after a refusal: the refused turn is the last again, and its buttons stay off.
        let mut f = Conversation::new(taken(SelectionKind::Writable, false));
        f.ask("q");
        f.open(AnswerPlace::Tray);
        f.end(&result(AskEnd::Refused, vec![]), false);
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
        assert_eq!(c.end(&write("**Пришлю** в четверг.", false), false), Heard::Write(PasteHow::Below));
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
        assert_eq!(c.end(&write("Пришлю.", false), false), Heard::Answered);
        c.open(AnswerPlace::Tray);
        assert_eq!((c.view.turns[0].text(), c.view.actions.as_slice()), ("Пришлю.".to_owned(), &[AnswerAction::Copy][..]));
        assert!(!c.has_write());

        let mut c = Conversation::new(taken(SelectionKind::Writable, true));
        c.ask("исправь");
        assert_eq!(c.end(&write("git push origin main", true), false), Heard::Answered, "no replace in a terminal: shown");
        assert_eq!(c.view.turns[0].text(), "git push origin main");

        // Decision 12: the request ended while a follow-up records — a write is shown, never pasted under the user's hands.
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("напиши письмо");
        assert_eq!(c.end(&write("Привет!", false), true), Heard::Answered);
        assert_eq!(c.paste_text().as_deref(), Some("Привет!"));
    }

    #[test]
    fn the_paste_of_an_answer_is_its_plain_text() {
        let mut c = Conversation::new(taken(SelectionKind::None, false));
        c.ask("q");
        assert_eq!(c.paste_text(), None, "nothing yet");
        c.step(Step::TextStart);
        c.step(Step::Text("**Коротко:** оба.\n* первый".into()));
        c.end(&result(AskEnd::Answer, vec![text_block("x")]), false);
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
        c.end(&result(AskEnd::Write { text: "x".into(), replace: true }, vec![call]), false);
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
        c.end(&result(AskEnd::Answer, vec![text_block("a")]), false);
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
        c.end(&result(AskEnd::Failed(AskFailure::Timeout), vec![]), false);
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
        d.end(&result(AskEnd::Failed(AskFailure::Timeout), vec![]), false);
        d.ask("а доллара");
        d.end(&result(AskEnd::Answer, vec![text_block("90")]), false);
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

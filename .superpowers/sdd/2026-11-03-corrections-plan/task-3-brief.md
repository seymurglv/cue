### Task 3: the offer's state and the Drop's actions

**Files:**
- Modify: `app/crates/core/src/corrections.rs`, `app/crates/core/src/ui.rs`

**Interfaces:**
- Produces in `corrections`: `#[derive(Clone, PartialEq, Eq, serde::Serialize)] pub struct SuggestView { pub word: String, pub missed: bool }`
  (hand-written `Debug`: `missed` and the word's length); `#[derive(Default)] pub struct Offers` with
  `offer(&mut self, r: Replacement)`, `at_rest(&mut self) -> bool` (true: the view changed, send it),
  `view(&self) -> Option<SuggestView>`, `dismissed(&mut self) -> bool`, `added(&mut self) -> Option<Replacement>`,
  `undone(&mut self) -> Option<Replacement>`, `done(&mut self) -> bool`, `dictation_started(&mut self) -> bool`.
- Produces in `ui`: `UiAction::{SuggestAdd, SuggestUndo, SuggestDismiss, SuggestDone}` (JSON `suggestAdd`,
  `suggestUndo`, `suggestDismiss`, `suggestDone`).

- [ ] **Step 1: write the failing tests** (in `corrections` tests):

```rust
    fn marisol() -> Replacement {
        Replacement { heard: "Марисоль".into(), word: "Marisol".into() }
    }
    fn shown(o: &Offers) -> Option<(String, bool)> {
        o.view().map(|v| (v.word, v.missed))
    }

    #[test]
    fn an_offer_waits_for_rest_then_shows() {
        let mut o = Offers::default();
        o.offer(marisol());
        assert_eq!(shown(&o), None, "waiting");
        assert!(o.at_rest());
        assert_eq!(shown(&o), Some(("Marisol".into(), false)));
        assert!(!o.at_rest(), "already shown");
    }

    #[test]
    fn a_dismissed_offer_is_missed_until_the_next_dictation() {
        let mut o = Offers::default();
        o.offer(marisol());
        o.at_rest();
        assert!(o.dismissed());
        assert_eq!(shown(&o), Some(("Marisol".into(), true)));
        assert!(o.dictation_started());
        assert_eq!(shown(&o), None);
        assert!(!o.at_rest(), "gone for good");
    }

    #[test]
    fn adding_then_undoing_gives_the_word_back_once() {
        let mut o = Offers::default();
        o.offer(marisol());
        o.at_rest();
        assert_eq!(o.added(), Some(marisol()));
        assert_eq!(o.added(), None, "once");
        assert_eq!(o.undone(), Some(marisol()));
        assert_eq!(shown(&o), None);
    }

    #[test]
    fn a_missed_offer_can_still_be_added_from_the_hover() {
        let mut o = Offers::default();
        o.offer(marisol());
        o.at_rest();
        o.dismissed();
        assert_eq!(o.added(), Some(marisol()));
        assert!(o.done());
        assert_eq!(shown(&o), None);
    }

    #[test]
    fn one_offer_at_a_time() {
        let mut o = Offers::default();
        o.offer(marisol());
        o.at_rest();
        o.offer(Replacement { heard: "гитхаб".into(), word: "GitHub".into() });
        assert_eq!(shown(&o), Some(("Marisol".into(), false)), "an open one stays");
        o.dismissed();
        o.offer(Replacement { heard: "гитхаб".into(), word: "GitHub".into() });
        assert_eq!(shown(&o), None, "a new one replaces a missed one and waits for rest");
        o.at_rest();
        assert_eq!(shown(&o), Some(("GitHub".into(), false)));
    }

    #[test]
    fn a_dictation_keeps_a_waiting_offer_and_drops_a_shown_one() {
        let mut o = Offers::default();
        o.offer(marisol());
        assert!(!o.dictation_started());
        o.at_rest();
        assert!(o.dictation_started());
        assert_eq!(shown(&o), None);
    }

    #[test]
    fn answers_out_of_turn_change_nothing() {
        let mut o = Offers::default();
        assert!(!o.dismissed() && !o.done());
        assert_eq!((o.added(), o.undone()), (None, None));
        o.offer(marisol());
        assert_eq!(o.added(), None, "not shown yet");
    }

    #[test]
    fn the_view_goes_to_the_page_as_word_and_missed() {
        let v = SuggestView { word: "Marisol".into(), missed: true };
        assert_eq!(serde_json::to_value(&v).unwrap(), serde_json::json!({ "word": "Marisol", "missed": true }));
        assert!(!format!("{v:?}").contains("Marisol"));
    }
```

And in `ui.rs`'s `every_variant_has_its_json_name` test add to the `actions` array:
`(UiAction::SuggestAdd, "suggestAdd"), (UiAction::SuggestUndo, "suggestUndo"), (UiAction::SuggestDismiss, "suggestDismiss"), (UiAction::SuggestDone, "suggestDone")`
(follow the array's existing shape exactly; if it lists names only, add the four names).

- [ ] **Step 2: run them and see them fail** — `cargo test -p cue-core`; expected: `Offers`, `SuggestView` and the
  four variants are not defined.

- [ ] **Step 3: implement `Offers`**

```rust
/// What the Drop shows of the suggestion: the word, and whether it was missed (the dot by the idle bar).
#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub struct SuggestView {
    pub word: String,
    pub missed: bool,
}

impl std::fmt::Debug for SuggestView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SuggestView").field("chars", &self.word.chars().count()).field("missed", &self.missed).finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Found while the Drop was busy: shown at the next rest.
    Waiting,
    /// The capsule with its 8 s ring.
    Open,
    /// The word went in; the capsule shows "undo" with its 6 s ring.
    Added,
    /// The ring ran out: the dot waits until the next dictation.
    Missed,
}

/// The one suggestion at a time.
#[derive(Default)]
pub struct Offers {
    cur: Option<(Replacement, Stage)>,
}

impl Offers {
    /// A new candidate: it replaces a waiting or a missed one; an open or just added one stays.
    pub fn offer(&mut self, r: Replacement) {
        if !matches!(self.cur, Some((_, Stage::Open | Stage::Added))) {
            self.cur = Some((r, Stage::Waiting));
        }
    }

    /// The Drop is at rest: a waiting suggestion opens. True when the view changed.
    pub fn at_rest(&mut self) -> bool {
        self.step(Stage::Waiting, Stage::Open)
    }

    pub fn view(&self) -> Option<SuggestView> {
        match &self.cur {
            Some((r, Stage::Open | Stage::Added)) => Some(SuggestView { word: r.word.clone(), missed: false }),
            Some((r, Stage::Missed)) => Some(SuggestView { word: r.word.clone(), missed: true }),
            _ => None,
        }
    }

    /// The 8 s ring ran out.
    pub fn dismissed(&mut self) -> bool {
        self.step(Stage::Open, Stage::Missed)
    }

    /// "+": the word to put into the dictionary, once.
    pub fn added(&mut self) -> Option<Replacement> {
        match &mut self.cur {
            Some((r, stage @ (Stage::Open | Stage::Missed))) => {
                *stage = Stage::Added;
                Some(r.clone())
            }
            _ => None,
        }
    }

    /// "Undo": the word to take out again; the suggestion is over.
    pub fn undone(&mut self) -> Option<Replacement> {
        match self.cur.take() {
            Some((r, Stage::Added)) => Some(r),
            other => {
                self.cur = other;
                None
            }
        }
    }

    /// The 6 s ring after adding ran out: the suggestion is over.
    pub fn done(&mut self) -> bool {
        let added = matches!(self.cur, Some((_, Stage::Added)));
        if added {
            self.cur = None;
        }
        added
    }

    /// The microphone opened for a dictation: a shown or missed suggestion goes (an added word stays in the
    /// dictionary); a waiting one keeps waiting. True when the view changed.
    pub fn dictation_started(&mut self) -> bool {
        let shown = matches!(self.cur, Some((_, Stage::Open | Stage::Added | Stage::Missed)));
        if shown {
            self.cur = None;
        }
        shown
    }

    fn step(&mut self, from: Stage, to: Stage) -> bool {
        match &mut self.cur {
            Some((_, stage)) if *stage == from => {
                *stage = to;
                true
            }
            _ => false,
        }
    }
}
```

In `ui.rs` add to `UiAction` after `TrayClose`, with doc comments:

```rust
    /// The suggestion's "+": the word goes into the dictionary.
    SuggestAdd,
    /// "Undo" after "+": the word comes out again.
    SuggestUndo,
    /// The suggestion's 8 s ring ran out: it waits as a dot.
    SuggestDismiss,
    /// The 6 s ring after "+" ran out.
    SuggestDone,
```

The runtime's `match` on `UiAction` has a catch-all arm today, so nothing else breaks; Task 7 adds their own arms.

- [ ] **Step 4: run** `cargo test -p cue-core` and `cargo clippy -p cue-core --all-targets -- -D warnings`: green.

---


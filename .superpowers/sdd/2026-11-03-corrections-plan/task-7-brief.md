### Task 7: the runtime — watch, offer, answers, settings to the windows

**Files:**
- Modify: `app/src-tauri/src/runtime.rs`, `app/src-tauri/src/ipc.rs`

**Interfaces:**
- Consumes: Tasks 1–6.
- Produces: `CoreEvent::Corrected { id: DictationId, pasted: String, span: Option<String> }`; `Ui::suggest(&self, v: Option<&SuggestView>)`
  emitting `cue://suggest` to the Drop; `Ui::settings(&self, v: &SettingsView)` emitting `cue://settings` to every
  window; `SettingsView.learned: Vec<LearnedView>` (`{ word, heard }`, camelCase) and `SettingsView.learn: bool`.

- [ ] **Step 1: `SettingsView` gets the learned words and the switch** (ipc.rs). Write the test first: in
  `settings_view_json_names` add `"learn"` and `"learned"` to the sorted key list and `("learn", json!(true))`,
  `("learned", json!([]))` to the defaults table; run `cargo test -p cue` (the app crate's name in
  `app/src-tauri/Cargo.toml`) and see it fail. Then add:

```rust
    /// The dictionary's words added from a suggestion, with how they were heard (the green book's tooltip).
    pub learned: Vec<LearnedView>,
    /// "Learn from me".
    pub learn: bool,
```

```rust
#[derive(Clone, PartialEq, serde::Serialize)]
pub struct LearnedView {
    pub word: String,
    pub heard: String,
}
```

In `SettingsView::of`: `learned: s.dictionary.learned.iter().map(|l| LearnedView { word: l.word.clone(), heard: l.heard.clone() }).collect(), learn: s.learn.enabled(),`.
In its hand-written `Debug`: `.field("learned", &self.learned.len()).field("learn", &self.learn)`. Run the test: passes.

- [ ] **Step 2: the two pushes in `Ui`** (ipc.rs, next to `tray`):

```rust
    /// The dictionary suggestion, to the Drop (docs/specs/2026-11-02-corrections.md); `None` folds it away.
    pub fn suggest(&self, v: Option<&cue_core::corrections::SuggestView>) {
        self.emit_to(DROP_LABEL, "cue://suggest", v);
    }

    /// The settings after a change the window did not ask for (a word added from the Drop), to every window.
    pub fn settings(&self, v: &SettingsView) {
        self.emit("cue://settings", v);
    }
```

(use the same `emit_to` helper `mic_names` uses; if it is a free function or takes another shape, copy that call).

- [ ] **Step 3: the runtime's state.** In `struct Runtime`, after `tray`:

```rust
    /// The dictionary suggestion from a correction (docs/specs/2026-11-02-corrections.md).
    offers: Offers,
    /// The dictation whose paste is awaited, with its text: the field is watched once it went in.
    watch_next: Option<(DictationId, String)>,
    /// Set to end the field watch early: a new dictation opened the microphone.
    field_stop: Option<Arc<AtomicBool>>,
```

initialized `Offers::default(), None, None` where the struct is built. Add `CoreEvent::Corrected { id: DictationId, pasted: String, span: Option<String> }`
with the doc comment "The field watch after a paste is over: `span` is the corrected text between the paste's anchors.
The text is never logged."

- [ ] **Step 4: start the watch after a dictation's paste.** In `fn insert` add, before `dispatch_insert`:

```rust
        self.watch_next = (self.settings.learn.enabled() && self.attempt.mode != Mode::Ask).then(|| (id, text.clone()));
```

In the `CoreEvent::InsertDone` arm, inside `if self.insert_watch.done(id)`, before `self.feed(…)`:

```rust
                    if outcome == InsertOutcome::Pasted {
                        self.watch_field(id);
                    }
```

```rust
    /// The field just pasted into is read until the user's correction is over (docs/specs/2026-11-02-corrections.md).
    fn watch_field(&mut self, id: DictationId) {
        let Some((wanted, pasted)) = self.watch_next.take() else { return };
        if wanted != id {
            return;
        }
        if let Some(old) = self.field_stop.take() {
            old.store(true, Ordering::Relaxed);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let tx = self.tx.clone();
        let text = pasted.clone();
        if fieldwatch::spawn(text, stop.clone(), move |span| {
            let _ = tx.send(CoreEvent::Corrected { id, pasted, span });
        }) {
            self.field_stop = Some(stop);
        }
    }
```

- [ ] **Step 5: a span becomes an offer; the offer shows at rest.**

```rust
            CoreEvent::Corrected { id: _, pasted, span } => self.on_corrected(&pasted, span.as_deref()),
```

```rust
    fn on_corrected(&mut self, pasted: &str, span: Option<&str>) {
        self.field_stop = None;
        let Some(span) = span else {
            tracing::info!("correction: none");
            return;
        };
        let found = corrections::candidates(pasted, span, &self.settings.dictionary.terms);
        tracing::info!(chars = span.chars().count(), candidates = found.len(), "correction found");
        if !self.settings.learn.enabled() {
            return;
        }
        if let Some(first) = found.into_iter().next() {
            self.offers.offer(first);
            // A missed one it replaced (the dot) is gone from the page until the new one opens at rest.
            self.ui.suggest(self.offers.view().as_ref());
            self.show_offer();
        }
    }

    /// A waiting suggestion opens once the Drop is at rest: idle, no tray card, no answer.
    fn show_offer(&mut self) {
        let rest = self.flow.ui_state().phase == UiPhase::Idle && self.tray.is_none() && !self.flow.answer_open();
        if rest && self.offers.at_rest() {
            self.ui.suggest(self.offers.view().as_ref());
        }
    }
```

Call `self.show_offer()` at the end of `feed` (after `sync_answer()`) and after `self.tray = None` in the `TrayClose`
arm (make that arm a block) and at the end of `on_tray_action`.

- [ ] **Step 6: a dictation drops a shown suggestion and ends the watch.** In `exec`, the `Effect::OpenMic { id }` arm
  becomes:

```rust
            Effect::OpenMic { id } => {
                if let Some(stop) = &self.field_stop {
                    stop.store(true, Ordering::Relaxed);
                }
                if self.offers.dictation_started() {
                    self.ui.suggest(None);
                }
                self.open_mic(id)
            }
```

- [ ] **Step 7: the page's answers.** Before the catch-all `CoreEvent::UiAction(a)` arm:

```rust
            CoreEvent::UiAction(UiAction::SuggestAdd) => {
                if let Some(r) = self.offers.added() {
                    let took = self.learn_word(SettingChange::DictLearn(LearnedWord { word: r.word, heard: r.heard }));
                    if !took {
                        self.offers.undone();
                        self.ui.suggest(None);
                    }
                }
            }
            CoreEvent::UiAction(UiAction::SuggestUndo) => {
                if let Some(r) = self.offers.undone() {
                    self.learn_word(SettingChange::DictRemove(r.word));
                }
                self.ui.suggest(None);
            }
            CoreEvent::UiAction(UiAction::SuggestDismiss) => {
                if self.offers.dismissed() {
                    self.ui.suggest(self.offers.view().as_ref());
                }
            }
            CoreEvent::UiAction(UiAction::SuggestDone) => {
                self.offers.done();
                self.ui.suggest(None);
            }
```

```rust
    /// A change from the Drop's suggestion: applied and written like the window's own, and the windows get the
    /// settings (the main window's Dictionary draws the word). False when the change was refused.
    fn learn_word(&mut self, change: SettingChange) -> bool {
        let took = apply_setting(&mut self.settings, change, &self.paths.settings, self.can_save_settings);
        if took {
            self.ui.settings(&SettingsView::of(&self.settings, self.lang, self.can_save_settings));
        }
        took
    }
```

In `UiReady`, after `self.ui.tray(self.tray);`: `self.ui.suggest(self.offers.view().as_ref());`.

When `Learn(false)` arrives through `change_setting`, end the watch and drop the suggestion: at the end of
`change_setting` add

```rust
        if !self.settings.learn.enabled() {
            if let Some(stop) = self.field_stop.take() {
                stop.store(true, Ordering::Relaxed);
            }
            if self.offers.dictation_started() {
                self.ui.suggest(None);
            }
            self.offers = Offers::default();
        }
```

- [ ] **Step 8: tests.** The runtime has no unit harness for these paths; the logic sits in `Offers` (Task 3) and
  `candidates` (Task 1). Add one ipc test: `Ui`-free, `serde_json::to_value(SettingsView::of(&s, Lang::En, true))`
  with a learned word gives `"learned":[{"word":"Marisol","heard":"Марисоль"}]`. Then `cargo test --workspace`,
  clippy: green; `cargo build -p cue` (app crate) builds.

---


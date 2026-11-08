### Task 10: documents, then the live check

**Files:**
- Modify: `README.md`, `docs/ARCHITECTURE.md`

- [ ] **Step 1: README.** In the privacy part, one paragraph: with "Learn from me" on, after a paste CUE reads the
  field it pasted into for up to a minute to notice a corrected word; the text stays in memory, nothing of it is
  written or sent; the word goes into the dictionary only when you press "+". Where the settings are listed, the
  switch on the General page.
- [ ] **Step 2: ARCHITECTURE.** The flow from paste to suggestion: `insert` → `InsertDone` → `fieldwatch::spawn`
  (`cue-field` thread, UI Automation) → `CoreEvent::Corrected` → `corrections::candidates` → `Offers` →
  `cue://suggest` → the Drop's capsule → `suggestAdd` → `SettingChange::DictLearn` → `cue://settings`; and that
  `fieldwatch`'s `FieldReader` is where another system plugs in its own reader.
- [ ] **Step 3: all checks** from the global constraints: green. `cargo run -p cue-cli -- leak-scan` (or the command
  `scripts/build-release.ps1 -ScanBuild` uses) still passes on a settings.json with a learned word.
- [ ] **Step 4: live, with the owner** (build the app the usual way, `npx tauri dev` from `app` or the release build):
  1. Claude (desktop): dictate "Мне нужно позвонить Марисоль", correct to "Marisol": the capsule comes ~2.5 s after the
     last keystroke; "+" adds it; the Dictionary shows it with the green book and the tooltip.
  2. Brave's address bar: the same with another name.
  3. Let the 8 s ring run out: the dot; hover the Drop: the capsule without a ring; start a dictation: the dot goes.
  4. "Undo" after "+": the word leaves the Dictionary.
  5. "Learn from me" off: no capsule after a correction.
  6. Telegram and a field inside a web page: note whether a capsule comes (the spec expects nothing where the field
     cannot be read; record what happens for the owner).
  7. Both themes.
- [ ] **Step 5: report to the owner** what passed, what did not, and ask whether to commit.

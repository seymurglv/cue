### Task 9: the History's chips

**Files:**
- Modify: `app/src-tauri/src/ipc.rs` (`EntryView`, line 171), `app/ui/src/window/api.ts` (`EntryView`, line 16),
  `app/ui/src/window/history.ts` (`details`, line 532), `app/ui/src/design/window.css`
- Test: `app/src-tauri/src/ipc.rs` tests, `app/ui/test/window-history.test.ts`

**Interfaces:**
- Consumes: `history::Entry.habits` (Task 2), filled by the runtime (Task 6).
- Produces: `EntryView.habits: Vec<history::HabitMark>` (JSON `habits: [{ id, short }]`, always present); TS
  `EntryView.habits: { id: string; short: string }[]`.

- [ ] **Step 1: the failing tests.** In `ipc.rs`'s `entry_view_has_the_time_of_day` add `"habits": [],` to the expected
  JSON; and

```rust
    /// The habits that shaped a dictation go to the window with it, by their short names; a log shows their count.
    #[test]
    fn entry_view_has_the_habits_that_shaped_it() {
        let e = history::Entry {
            id: "2026-11-04T10:00:00.000-1".into(),
            ts: "2026-11-04T10:00:00.000".into(),
            mode: history::Mode::Dictation,
            app: "Telegram Desktop".into(),
            text: "скинь файл".into(),
            question: None,
            raw: Some("Скинь файл".into()),
            habits: vec![history::HabitMark { id: "h1".into(), short: "без точки".into() }],
        };
        let view = serde_json::to_value(EntryView::of(&e, false)).unwrap();
        assert_eq!(view["habits"], json!([{ "id": "h1", "short": "без точки" }]));
        assert!(!format!("{:?}", EntryView::of(&e, false)).contains("точки"));
    }
```

  In `window-history.test.ts`, in "How it was said":

```ts
  it('an entry a habit shaped lists it under how it was said, by its short name', async () => {
    const shaped = entry(`${TODAY}T11:00:00.000-1`, 'send the file', { app: 'Telegram', raw: 'Send the file.', habits: [{ id: 'h1', short: 'no full stop' }] });
    core.historyPage.mockResolvedValue(pageOf(shaped, said));
    page = await mount();
    openRow(shaped.id);
    const box = rowOf(shaped.id).querySelector('.hdet .hhabs')!;
    expect(box.getAttribute('aria-label')).toBe('Habits applied');
    const chips = [...box.querySelectorAll('.hhab')];
    expect(chips.map((c) => text(c))).toEqual(['no full stop']);
    expect(chips[0].querySelector('svg')).not.toBeNull();
    openRow(said.id);
    expect(rowOf(said.id).querySelector('.hdet .hhabs')).toBeNull();
  });
```

  Run `cargo test -p cue ipc` and `npx vitest run test/window-history.test.ts`: they fail.

- [ ] **Step 2: implement.** `EntryView` gains `pub habits: Vec<history::HabitMark>,` (doc: "…and the habits whose
  effect showed in its text, by their short names"), `of` sets `habits: e.habits.clone()`, its `Debug` adds
  `.field("habits", &self.habits.len())`. TS `EntryView` gains
  `/** The habits whose effect showed in its text, by their short names as they were then. */ habits: { id: string; short: string }[];`.
  In `history.ts`'s `details`, before `inner.append(this.player(e).el);`:

```ts
    if (e.habits?.length) inner.append(this.habitChips(e));
```

  and the method after `details`:

```ts
  /** The habits that shaped the entry's text (docs/specs/2026-11-04-habits.md): a chip each, the habit icon and its
   *  short name as it was then (a forgotten habit keeps its chip). */
  private habitChips(e: EntryView): HTMLElement {
    const box = h('div', 'hhabs');
    box.setAttribute('role', 'list');
    box.setAttribute('aria-label', this.ctx.t.habitsApplied);
    for (const m of e.habits) {
      const chip = h('span', 'hhab');
      chip.setAttribute('role', 'listitem');
      chip.innerHTML = icon('habit');
      chip.append(m.short);
      box.append(chip);
    }
    return box;
  }
```

  (`icon` is imported in `history.ts`; if not, import it from `./icons`). `window.css`, next to `.hraw`:

```css
/* The habits that shaped an entry: small chips under how it was said. */
.hhabs{display:flex;flex-wrap:wrap;gap:6px;margin-top:8px}
.hhab{display:inline-flex;align-items:center;gap:4px;height:22px;padding:0 8px;border-radius:999px;font-size:11px;line-height:1;color:var(--text);background:color-mix(in srgb,var(--ok) 12%,transparent)}
.hhab svg{width:12px;height:12px;color:var(--ok)}
```

- [ ] **Step 3: run** `npx vitest run`, `npx tsc --noEmit -p tsconfig.json`, `cargo test --workspace`: green.

---


- Task 8 left `entry()` in app/ui/test/windowFixture.ts without `habits: []` because `EntryView` had no such field yet: add both in this task.

- Task 8 added the text key `habitsApplied` in app/ui/src/window/text.ts for this task: use it (or remove it if this task needs a different shape).

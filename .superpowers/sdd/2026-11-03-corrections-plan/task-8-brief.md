### Task 8: the Drop's capsule and dot

**Files:**
- Modify: `app/ui/src/design/drop.js`, `app/ui/src/design/drop.css`, `app/ui/src/controller.ts`, `app/ui/src/strings.ts`,
  `app/ui/src/main.ts`
- Test: `app/ui/test/controller.test.ts`

**Interfaces:**
- Consumes: `cue://suggest` payload `{ word: string; missed: boolean } | null`; actions `suggestAdd`, `suggestUndo`,
  `suggestDismiss`, `suggestDone` through `io.send`.
- Produces: `DropController.suggest(v: Suggest | null)`, `export interface Suggest { word: string; missed: boolean }`;
  drop.js `Drop.offer(word, missed)`, `Drop.unoffer()`, states `sugg` and `added`, `api.onSuggest(kind)` with
  kind `'add' | 'undo' | 'expire'`.

- [ ] **Step 1: the failing tests** (new `describe` in controller.test.ts, using the file's existing `ctl`, `sent`,
  `rectsSent`):

```ts
describe('Dictionary suggestion', () => {
  const drop = () => document.querySelector('.drop') as HTMLElement;
  const btn = () => drop().querySelector('.sbt') as HTMLButtonElement;
  const dot = () => document.querySelector('.sgdot') as HTMLElement;
  const ring = () => btn().querySelector('.ring circle') as SVGElement;
  const ringOut = () => ring().dispatchEvent(new Event('animationend', { bubbles: true }));

  it('opens the capsule with the word and adds it', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    expect(drop().dataset.s).toBe('sugg');
    expect(drop().querySelector('.swd')!.textContent).toContain('Marisol');
    expect(btn().getAttribute('aria-label')).toBe('Add to dictionary');
    btn().click();
    expect(sent).toEqual(['suggestAdd']);
    expect(drop().dataset.s).toBe('added');
    expect(drop().querySelector('.sbk')!.classList.contains('ok')).toBe(true);
    expect(btn().getAttribute('aria-label')).toBe('Undo');
  });

  it('undo takes the word out; the end of its ring closes', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    btn().click();
    btn().click();
    expect(sent).toEqual(['suggestAdd', 'suggestUndo']);
    ctl.suggest(null); // the core answers an undo with nothing
    expect(drop().dataset.s).toBe('idle');
    ctl.suggest({ word: 'Marisol', missed: false });
    btn().click();
    ringOut();
    expect(sent.at(-1)).toBe('suggestDone');
    expect(drop().dataset.s).toBe('idle');
  });

  it('a ring run out tells the core, and the missed one waits as a dot', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ringOut();
    expect(sent).toEqual(['suggestDismiss']);
    ctl.suggest({ word: 'Marisol', missed: true });
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(true);
  });

  it('hovering a missed one opens the capsule without a ring, leaving folds it back', () => {
    ctl.suggest({ word: 'Marisol', missed: true });
    drop().dispatchEvent(new MouseEvent('mouseenter'));
    expect(drop().dataset.s).toBe('sugg');
    expect(drop().classList.contains('sgwait')).toBe(true);
    drop().dispatchEvent(new MouseEvent('mouseleave'));
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(true);
  });

  it('nothing folds the capsule away; a dictation drops it locally too', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ctl.suggest(null);
    expect(drop().dataset.s).toBe('idle');
    expect(dot().classList.contains('on')).toBe(false);
    ctl.suggest({ word: 'Marisol', missed: true });
    ctl.apply({ phase: 'start', error: null, retry: false, paused: false, mode: 'dictate' });
    expect(dot().classList.contains('on')).toBe(false);
  });

  it('an idle state while the capsule is open keeps it open', () => {
    ctl.suggest({ word: 'Marisol', missed: false });
    ctl.apply({ phase: 'idle', error: null, retry: false, paused: false, mode: 'dictate' });
    expect(drop().dataset.s).toBe('sugg');
  });

  it('the word goes in as text', () => {
    ctl.suggest({ word: '<b>x</b>', missed: false });
    expect(drop().querySelector('.swd b')).toBeNull();
  });
});
```

Add the Russian names to the strings test if it checks key parity (`controller.test.ts:47` does: new keys go into both).

- [ ] **Step 2: run and see them fail** — from `app`: `npx vitest run test/controller.test.ts`.

- [ ] **Step 3: drop.js.** In the constructor, after the `ok` part:

```js
  P('sbk',`<span class="mi sbk">${IC.book}</span>`);P('swd','<span class="lab swd"></span>');
  P('sbt','<button class="ib solid sbt"></button>');
  this.dot=el('<i class="sgdot" aria-hidden="true"></i>');this.root.append(this.dot);
  this.word='';this.missed=false;this.peek=false;
```

Add `'swd'` to the `TXT` set (the text parts that open to their scrollWidth). In the `if(this.live)` block:

```js
   this.parts.sbt.addEventListener('click',()=>{if(this.s==='sugg'){this.missed=false;this.peek=false;api.onSuggest('add');this.enter('added')}else if(this.s==='added')api.onSuggest('undo')});
   this.parts.sbt.addEventListener('animationend',e=>{if(!e.target.closest||!e.target.closest('.ring'))return;
    if(this.s==='sugg'&&!this.peek){api.onSuggest('expire');this.enter('idle')}else if(this.s==='added'){api.onSuggest('done');this.unoffer()}});
   this.p.addEventListener('mouseenter',()=>{if(this.missed&&this.s==='idle'){this.peek=true;this.enter('sugg')}});
   this.p.addEventListener('mouseleave',()=>{if(this.peek&&this.s==='sugg'){this.peek=false;this.enter('idle')}});
```

(The existing mouseenter/mouseleave that set `hov` stay; both run.) Methods:

```js
 /* The dictionary suggestion (docs/specs/2026-11-02-corrections.md): open — the capsule with its 8 s ring; missed —
    the dot by the idle bar, and hovering opens the capsule without a ring. */
 offer(word,missed){this.word=word;this.missed=!!missed;this.peek=false;
  if(this.missed){if(this.s==='sugg'||this.s==='added')this.enter('idle');else this.layout()}
  else if(this.s==='idle'||this.s==='sugg')this.enter('sugg');else this.layout()}
 unoffer(){this.word='';this.missed=false;this.peek=false;if(this.s==='sugg'||this.s==='added')this.enter('idle');else this.layout()}
```

In `enter(s)`: at the top `if(s==='start'){this.missed=false;this.peek=false}`; for `sugg`/`added` skip the morph's
state change (`this.morph.set('idle',now)`), set the button:

```js
  if(s==='sugg'||s==='added'){const T=this.api.text,add=s==='sugg',t=add?T.learnAdd:T.learnUndo;
   this.parts.sbt.innerHTML=RING+(add?IC.plus:IC.undo);this.parts.sbt.title=t;this.parts.sbt.setAttribute('aria-label',t);
   CUE.setLabel(this.parts.swd,this.word)}
```

(`CUE.setLabel` builds spans from characters, so the word stays text; check `IC.plus` exists in drop.js's `IC`, else
add `plus:SV('<path d="M10 5v10M5 10h10"/>')` in the same style as the other icons.) In `layout()`'s two `V` tables
add `sugg:{sbk:1,swd:1,sbt:1},added:{sbk:1,swd:1,sbt:1}`; at the end of `layout()`:

```js
  this.parts.sbk.classList.toggle('ok',s==='added');
  this.p.classList.toggle('sgwait',s==='sugg'&&this.peek);
  this.dot.classList.toggle('on',s==='idle'&&this.missed);
```

- [ ] **Step 4: drop.css** (after the `.ib` rules; values from the approved mockup, design stage 3):

```css
/* The dictionary suggestion: the Drop's capsule book · word · button, and the dot of a missed one. */
.drop .sbk{transition:color .3s}
.drop .sbk.ok{color:var(--ok)}
.drop[data-s=sugg] .sbt .ring circle{animation:deplete 8s linear forwards}
.drop[data-s=added] .sbt .ring circle{animation:deplete 6s linear forwards}
.drop:hover .sbt .ring circle{animation-play-state:paused}
.drop.sgwait .sbt .ring{display:none}
.sgdot{position:absolute;left:calc(100% + 5px);bottom:1px;width:5px;height:5px;border-radius:50%;background:var(--ok);
  box-shadow:0 0 0 1.5px rgba(18,18,20,.9);opacity:0;transform:scale(0);transition:transform .4s var(--e-spring),opacity .2s}
.sgdot.on{opacity:1;transform:scale(1)}
@media (prefers-reduced-motion:reduce){.drop .sbt .ring circle{animation-duration:.01s}.sgdot{transition:none}}
```

Check where `.pv` is positioned so `.sgdot` sits by the idle bar (it is appended to `this.root`, the `.pv`); if the
reduced-motion rule would make the 8 s ring end at once, drop that line instead and leave the ring as it is (the
existing cancel ring is the model to follow: copy what it does under reduced motion).

- [ ] **Step 5: strings.ts.** In `DropText` add `learnAdd: string; learnUndo: string;` with doc comments; `en`:
  `learnAdd: 'Add to dictionary', learnUndo: 'Undo'`; `ru`: `learnAdd: 'Добавить в словарь', learnUndo: 'Отменить'`.

- [ ] **Step 6: controller.ts.**

```ts
/** The dictionary suggestion from the core (cue://suggest): the word, and whether it was missed (the dot). */
export interface Suggest { word: string; missed: boolean }
```

`UiAction` gains `| 'suggestAdd' | 'suggestUndo' | 'suggestDismiss' | 'suggestDone'`. In the Drop `api` object:

```ts
      onSuggest: (k: 'add' | 'undo' | 'expire' | 'done') => {
        this.io.send(k === 'add' ? 'suggestAdd' : k === 'undo' ? 'suggestUndo' : k === 'expire' ? 'suggestDismiss' : 'suggestDone');
        this.wake();
      },
```

```ts
  /** The core's suggestion: shown only on the idle Drop (the core sends it at rest); null folds it away. */
  suggest(v: Suggest | null) {
    if (v) this.drop.offer(v.word, v.missed);
    else this.drop.unoffer();
    this.wake();
  }
```

In `apply(s)`, where it calls `this.drop.enter(...)`: an idle state must not fold an open capsule —
`const keep = s.phase === 'idle' && (this.drop.s === 'sugg' || this.drop.s === 'added');` and skip the `enter` when
`keep` (call `this.drop.layout()` instead). In `frame()`'s `calm`: add `&& this.drop.s !== 'sugg' && this.drop.s !== 'added'`
so the rectangles follow the capsule while it grows.

- [ ] **Step 7: main.ts.** Next to the `cue://tray` listener:
  `await listen<Suggest | null>('cue://suggest', (e) => ctl.suggest(e.payload));` (same shape as the others there).

- [ ] **Step 8: run** `npx vitest run` and `npx tsc --noEmit -p tsconfig.json` from `app`: green.

---


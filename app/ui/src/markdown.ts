// Safe Markdown for "Ask" answers (spec 2 §7): paragraphs, "- " and "1. " lists, **bold**, *italic*, `code`, fenced code,
// headings shown as bold lines, links as their text, citation marks. No HTML: every piece of the model's text reaches the
// page as a text node, never as markup.

/** A citation mark in the answer's text: U+E000, the source number, U+E001. Private-use characters; the model's own are
 *  taken out before the marks go in (answer.ts, markedText), so no answer can fake one. */
export const CITE_OPEN = '';
export const CITE_CLOSE = '';

export type Inline =
  | { t: 'text'; v: string }
  | { t: 'b'; c: Inline[] }
  | { t: 'i'; c: Inline[] }
  | { t: 'code'; v: string }
  | { t: 'cite'; n: number };

export type Block =
  | { t: 'p'; c: Inline[] }
  | { t: 'ul'; items: Inline[][] }
  | { t: 'ol'; start: number; items: Inline[][] }
  | { t: 'pre'; v: string };

const FENCE = /^\s*```/;
const FENCE_END = /^\s*```\s*$/;
const BULLET = /^\s*[-*+]\s+(.*)$/;
const NUMBERED = /^\s*(\d{1,9})[.)]\s+(.*)$/;
const HEADING = /^\s*#{1,6}\s+(.*)$/;
/** Every citation mark in a run of text. */
const CITES = new RegExp(`${CITE_OPEN}(\\d{1,4})${CITE_CLOSE}`, 'g');

/** Code and a link's address are not markup, so a citation mark that a segment's end put there is no citation in place: it
 *  comes out of the text, and its number follows the code or the link. */
function liftCites(s: string): { text: string; cites: Inline[] } {
  const cites: Inline[] = [];
  const text = s.replace(CITES, (_, n: string) => {
    cites.push({ t: 'cite', n: Number(n) });
    return '';
  });
  return { text, cites };
}

/** The answer's blocks, line by line. A fence still open runs to the end: the answer may still be streaming. */
export function parseMarkdown(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, '\n').split('\n');
  const out: Block[] = [];
  let para: string[] = [];
  const flush = () => {
    if (para.length) out.push({ t: 'p', c: parseInline(para.join('\n')) });
    para = [];
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (FENCE.test(line)) {
      flush();
      // Marks on the opening line, in the code or on the closing line (which still closes it) follow the block.
      const cites = liftCites(line).cites;
      const code: string[] = [];
      for (i++; i < lines.length; i++) {
        const { text, cites: more } = liftCites(lines[i]);
        cites.push(...more);
        if (FENCE_END.test(text)) break;
        code.push(text);
      }
      out.push({ t: 'pre', v: code.join('\n') });
      if (cites.length) out.push({ t: 'p', c: cites });
      continue;
    }
    const bullet = BULLET.exec(line);
    const numbered = bullet ? null : NUMBERED.exec(line);
    if (bullet || numbered) {
      flush();
      const item = parseInline(bullet ? bullet[1] : numbered![2]);
      const last = out[out.length - 1];
      if (bullet && last?.t === 'ul') last.items.push(item);
      else if (numbered && last?.t === 'ol') last.items.push(item);
      else if (bullet) out.push({ t: 'ul', items: [item] });
      else out.push({ t: 'ol', start: Number(numbered![1]), items: [item] });
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      flush();
      out.push({ t: 'p', c: [{ t: 'b', c: parseInline(heading[1]) }] });
      continue;
    }
    if (line.trim()) para.push(line);
    else flush();
  }
  flush();
  return out;
}

/** Inline markup of a block; anything unclosed stays as written. `*` between letters or digits ("2*3*4") is not italic. */
export function parseInline(s: string): Inline[] {
  const out: Inline[] = [];
  let text = '';
  const push = () => {
    if (text) out.push({ t: 'text', v: text });
    text = '';
  };
  for (let i = 0; i < s.length; ) {
    const rest = s.slice(i);
    let m: RegExpExecArray | null;
    if ((m = /^(\d{1,4})/.exec(rest))) {
      push();
      out.push({ t: 'cite', n: Number(m[1]) });
    } else if ((m = /^`([^`\n]+)`/.exec(rest))) {
      push();
      const { text: code, cites } = liftCites(m[1]);
      if (code) out.push({ t: 'code', v: code });
      out.push(...cites);
    } else if ((m = /^\*\*(?=\S)([\s\S]*?\S)\*\*/.exec(rest))) {
      push();
      out.push({ t: 'b', c: parseInline(m[1]) });
    } else if ((m = /^\*(?=[^\s*])([^*\n]*?[^\s*])\*/.exec(rest)) && !/[\p{L}\p{N}]/u.test(s[i - 1] ?? '')) {
      push();
      out.push({ t: 'i', c: parseInline(m[1]) });
    } else if ((m = /^\[([^\]\n]+)\]\(([^)\s]+)\)/.exec(rest))) {
      push();
      out.push(...parseInline(m[1]), ...liftCites(m[2]).cites);
    } else {
      text += s[i];
      i++;
      continue;
    }
    i += m[0].length;
  }
  push();
  return out;
}

/** The blocks as DOM, and the answer's words in reading order: every run of non-space characters is a <span class="w">,
 *  the unit the answer is revealed by, and so is a citation mark. Text goes in as text nodes only. */
export function renderBlocks(blocks: Block[], doc: Document = document): { root: DocumentFragment; words: HTMLElement[] } {
  const words: HTMLElement[] = [];
  const root = doc.createDocumentFragment();
  const text = (parent: Node, v: string) => {
    v.split('\n').forEach((line, k) => {
      if (k > 0) parent.appendChild(doc.createElement('br'));
      for (const part of line.split(/(\s+)/)) {
        if (!part) continue;
        if (/^\s+$/.test(part)) {
          parent.appendChild(doc.createTextNode(part));
          continue;
        }
        const w = doc.createElement('span');
        w.className = 'w';
        w.textContent = part;
        parent.appendChild(w);
        words.push(w);
      }
    });
  };
  const inline = (parent: Node, items: Inline[]) => {
    for (const it of items) {
      if (it.t === 'text') text(parent, it.v);
      else if (it.t === 'code') {
        const c = doc.createElement('code');
        text(c, it.v);
        parent.appendChild(c);
      } else if (it.t === 'cite') {
        const c = doc.createElement('sup');
        c.className = 'cite w';
        c.dataset.n = String(it.n);
        c.textContent = String(it.n);
        parent.appendChild(c);
        words.push(c);
      } else {
        const el = doc.createElement(it.t);
        inline(el, it.c);
        parent.appendChild(el);
      }
    }
  };
  for (const b of blocks) {
    if (b.t === 'p') {
      const p = doc.createElement('p');
      inline(p, b.c);
      root.appendChild(p);
    } else if (b.t === 'pre') {
      const pre = doc.createElement('pre');
      const code = doc.createElement('code');
      text(code, b.v);
      pre.appendChild(code);
      root.appendChild(pre);
    } else {
      const list = doc.createElement(b.t);
      if (b.t === 'ol' && b.start !== 1) list.setAttribute('start', String(b.start));
      for (const item of b.items) {
        const li = doc.createElement('li');
        inline(li, item);
        list.appendChild(li);
      }
      root.appendChild(list);
    }
  }
  return { root, words };
}

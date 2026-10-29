import { describe, expect, it } from 'vitest';
import { CITE_CLOSE, CITE_OPEN, parseInline, parseMarkdown, renderBlocks } from '../src/markdown';

const html = (src: string) => {
  const { root } = renderBlocks(parseMarkdown(src));
  const d = document.createElement('div');
  d.appendChild(root);
  return d;
};

describe('Safe Markdown of answers', () => {
  it('paragraphs keep their line breaks; a heading is a bold line', () => {
    expect(parseMarkdown('First line\nsecond line\n\n## Next\nThird')).toEqual([
      { t: 'p', c: [{ t: 'text', v: 'First line\nsecond line' }] },
      { t: 'p', c: [{ t: 'b', c: [{ t: 'text', v: 'Next' }] }] },
      { t: 'p', c: [{ t: 'text', v: 'Third' }] },
    ]);
    expect(html('a\nb').querySelectorAll('p br')).toHaveLength(1);
  });

  it('bold, italic, code, citations; a link keeps its text only; unclosed markup stays as written', () => {
    expect(parseInline(`**Коротко:** *можно* \`docker swarm init\`${CITE_OPEN}2${CITE_CLOSE} [docs](https://x.io)`)).toEqual([
      { t: 'b', c: [{ t: 'text', v: 'Коротко:' }] },
      { t: 'text', v: ' ' },
      { t: 'i', c: [{ t: 'text', v: 'можно' }] },
      { t: 'text', v: ' ' },
      { t: 'code', v: 'docker swarm init' },
      { t: 'cite', n: 2 },
      { t: 'text', v: ' ' },
      { t: 'text', v: 'docs' },
    ]);
    expect(parseInline('2*3*4 and **unclosed')).toEqual([{ t: 'text', v: '2*3*4 and **unclosed' }]);
    expect(html('[docs](https://x.io)').querySelector('a')).toBeNull();
  });

  it('lists: bullets of any sign, numbers from their own start, one list across a blank line', () => {
    expect(parseMarkdown('- a\n* b\n\n- c\n3. x\n4. y')).toEqual([
      { t: 'ul', items: [[{ t: 'text', v: 'a' }], [{ t: 'text', v: 'b' }], [{ t: 'text', v: 'c' }]] },
      { t: 'ol', start: 3, items: [[{ t: 'text', v: 'x' }], [{ t: 'text', v: 'y' }]] },
    ]);
    expect(html('3. x').querySelector('ol')!.getAttribute('start')).toBe('3');
    expect(html('1. x').querySelector('ol')!.hasAttribute('start')).toBe(false);
  });

  it('code fences keep their lines; one still open runs to the end of a streaming answer', () => {
    expect(parseMarkdown('Run:\n```bash\npip install -r requirements.txt\n```\nDone')).toEqual([
      { t: 'p', c: [{ t: 'text', v: 'Run:' }] },
      { t: 'pre', v: 'pip install -r requirements.txt' },
      { t: 'p', c: [{ t: 'text', v: 'Done' }] },
    ]);
    expect(parseMarkdown('```\nls\n  -la')).toEqual([{ t: 'pre', v: 'ls\n  -la' }]);
  });

  it('words: every run of letters is a span, whitespace stays, a citation is a word of its own', () => {
    const { root, words } = renderBlocks(parseMarkdown(`Rust  1.95${CITE_OPEN}1${CITE_CLOSE}.`));
    expect(words.map((w) => w.textContent)).toEqual(['Rust', '1.95', '1', '.']);
    expect(words[2].className).toBe('cite w');
    expect(words[2].dataset.n).toBe('1');
    const d = document.createElement('div');
    d.appendChild(root);
    expect(d.textContent).toBe('Rust  1.951.');
  });

  it('HTML in an answer is text, never markup', () => {
    const d = html('<img src=x onerror="alert(1)"> and <script>alert(1)</script>');
    expect(d.querySelector('img, script')).toBeNull();
    expect(d.textContent).toBe('<img src=x onerror="alert(1)"> and <script>alert(1)</script>');
  });
});

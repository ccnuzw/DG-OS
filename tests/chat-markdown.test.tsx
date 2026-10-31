import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Lexer } from 'marked';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ModalHost } from '@dgos/ui';
import Markdown from '../src/components/chat/Markdown';

// AC14 (highlight) and AC29 (safe rendering). Synthetic content only.
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
const opened = vi.mocked(openUrl);

const show = (source: string) => render(<><Markdown source={source} /><ModalHost /></>);

beforeEach(() => { opened.mockResolvedValue(undefined); });
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('raw HTML is inert text', () => {
  it('<script> blocks are shown as text and create no element', () => {
    const { container } = show('before\n\n<script>window.pwned = 1</script>\n\nafter');
    expect(container.querySelector('script')).toBeNull();
    expect(container.textContent).toContain('<script>window.pwned = 1</script>');
    expect((window as unknown as { pwned?: number }).pwned).toBeUndefined();
  });

  it('<img onerror> never becomes an element or an attribute', () => {
    const { container } = show('x <img src="https://remote.invalid/a.png" onerror="window.pwned=1"> y');
    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('[onerror]')).toBeNull();
    expect(container.textContent).toContain('onerror="window.pwned=1"');
  });

  it('nested HTML keeps every tag as text', () => {
    const { container } = show('<div><p onclick="x()"><b>bold</b><iframe src="https://remote.invalid"></iframe></p></div>');
    expect(container.querySelector('div > p[onclick], iframe, b')).toBeNull();
    expect(container.textContent).toContain('<iframe src="https://remote.invalid"></iframe>');
  });

  it('inline HTML inside a sentence stays text', () => {
    const { container } = show('a <b>b</b> and <a href="javascript:x()">c</a> end');
    expect(container.querySelector('b, a')).toBeNull();
    expect(container.textContent).toContain('<a href="javascript:x()">');
  });
});

describe('links', () => {
  it('javascript: and data: links are text, not controls', () => {
    const { container } = show('[run](javascript:alert(1)) and [blob](data:text/html;base64,PHNjcmlwdD4=)');
    expect(container.querySelector('a')).toBeNull();
    expect(screen.queryByRole('button')).toBeNull();
    expect(screen.queryByRole('link')).toBeNull();
    expect(container.textContent).toContain('run (javascript:alert(1))');
    expect(container.textContent).toContain('blob (data:text/html;base64,PHNjcmlwdD4=)');
  });

  it.each(['http://insecure.invalid/x', 'file:///etc/hosts', 'mailto:a@b.invalid', 'vbscript:x', '//protocol-relative.invalid/x'])(
    'a %s link is not interactive', href => {
      const { container } = show(`[go](${href})`);
      expect(container.querySelector('a')).toBeNull();
      expect(screen.queryByRole('button', { name: 'go' })).toBeNull();
      expect(container.textContent).toContain(`go (${href})`);
    },
  );

  it('an https link asks for confirmation before opening, and never opens on cancel', async () => {
    show('See [the docs](https://docs.example.invalid/page?a=1).');
    fireEvent.click(screen.getByRole('button', { name: 'the docs' }));
    const dialog = await screen.findByRole('dialog', { name: '打开外部链接？' });
    expect(within(dialog).getByText('https://docs.example.invalid/page?a=1')).toBeTruthy();
    expect(opened).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole('button', { name: '取消' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(opened).not.toHaveBeenCalled();
  });

  it('confirming opens the exact normalized URL once', async () => {
    show('[the docs](https://docs.example.invalid/page)');
    fireEvent.click(screen.getByRole('button', { name: 'the docs' }));
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(within(dialog).getByRole('button', { name: '打开' }));
    await waitFor(() => expect(opened).toHaveBeenCalledTimes(1));
    expect(opened).toHaveBeenCalledWith('https://docs.example.invalid/page');
  });

  it('an autolink is shown as text because the user chose nothing to click', () => {
    show('<https://auto.example.invalid>');
    expect(screen.queryByRole('button')).toBeNull();
  });
});

describe('images never load', () => {
  it('a remote image renders alt text and the URL as text only', () => {
    const { container } = show('![a chart](https://remote.invalid/track.png?q=secret)');
    expect(container.querySelector('img')).toBeNull();
    expect(container.textContent).toContain('[图片：a chart]');
    expect(container.textContent).toContain('https://remote.invalid/track.png?q=secret');
  });

  it('http and data images are text too', () => {
    const { container } = show('![x](http://remote.invalid/a.png) ![y](data:image/png;base64,AAAA)');
    expect(container.querySelector('img')).toBeNull();
  });
});

describe('code blocks', () => {
  it('highlights a known language with hljs-* classes only', () => {
    const { container } = show('```python\nprint(1)  # note\n```');
    const spans = [...container.querySelectorAll('pre code span')];
    expect(spans.length).toBeGreaterThan(0);
    for (const span of spans) expect(span.className).toMatch(/^hljs-[a-z_-]+$/);
    expect(container.querySelector('pre code')?.textContent).toBe('print(1)  # note');
  });

  it('an unknown language renders plain text without throwing', () => {
    const { container } = show('```not-a-language\n<b>x</b> & y\n```');
    expect(container.querySelector('pre code span')).toBeNull();
    expect(container.querySelector('pre code b')).toBeNull();
    expect(container.querySelector('pre code')?.textContent).toBe('<b>x</b> & y');
  });

  it('a fence without a language renders plain text', () => {
    const { container } = show('```\nplain\n```');
    expect(container.querySelector('pre code')?.textContent).toBe('plain');
  });

  it('highlighted code containing HTML stays text', () => {
    const { container } = show('```html\n<script>alert(1)</script>\n```');
    expect(container.querySelector('script')).toBeNull();
    expect(container.querySelector('pre code')?.textContent).toBe('<script>alert(1)</script>');
  });
});

describe('copy code block', () => {
  const original = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
  afterEach(() => {
    if (original) Object.defineProperty(navigator, 'clipboard', original);
    else Reflect.deleteProperty(navigator, 'clipboard');
    Reflect.deleteProperty(document, 'execCommand');
  });

  it('writes the code through navigator.clipboard and says so', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    show('```js\nconst a = 1;\n```');
    fireEvent.click(screen.getByRole('button', { name: '复制代码' }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('const a = 1;'));
    expect(await screen.findByText('已复制')).toBeTruthy();
  });

  it('falls back to execCommand when the clipboard API rejects', async () => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } });
    const exec = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, 'execCommand', { configurable: true, value: exec });
    show('```js\nconst a = 1;\n```');
    fireEvent.click(screen.getByRole('button', { name: '复制代码' }));
    expect(await screen.findByText('已复制')).toBeTruthy();
    expect(exec).toHaveBeenCalledWith('copy');
  });

  it('shows a visible failure when both paths fail', async () => {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } });
    Object.defineProperty(document, 'execCommand', { configurable: true, value: vi.fn().mockReturnValue(false) });
    show('```js\nconst a = 1;\n```');
    fireEvent.click(screen.getByRole('button', { name: '复制代码' }));
    expect((await screen.findByText(/复制失败/)).textContent).toContain('手动');
  });
});

describe('render failure falls back to plain text', () => {
  beforeEach(() => { vi.spyOn(console, 'error').mockImplementation(() => undefined); });

  it('a lexer exception shows the raw content as text', () => {
    vi.spyOn(Lexer, 'lex').mockImplementation(() => { throw new Error('boom'); });
    const { container } = show('# Title\n\n**raw** <b>kept</b>');
    expect(container.querySelector('pre')?.textContent).toBe('# Title\n\n**raw** <b>kept</b>');
    expect(container.querySelector('h1, strong, b')).toBeNull();
  });

  it('an exception thrown while rendering a token also falls back', () => {
    const poisoned = Object.assign([{ type: 'paragraph', raw: 'x', text: 'x' }], { links: {} });
    Object.defineProperty(poisoned[0], 'tokens', { get() { throw new Error('bad token'); } });
    vi.spyOn(Lexer, 'lex').mockReturnValue(poisoned as unknown as ReturnType<typeof Lexer.lex>);
    const { container } = show('raw fallback text');
    expect(container.querySelector('pre')?.textContent).toBe('raw fallback text');
  });

  it('recovers when the content changes to something that renders', () => {
    const lex = vi.spyOn(Lexer, 'lex').mockImplementation(() => { throw new Error('boom'); });
    const view = show('first');
    expect(view.container.querySelector('pre')?.textContent).toBe('first');
    lex.mockRestore();
    view.rerender(<><Markdown source="**second**" /><ModalHost /></>);
    expect(view.container.querySelector('strong')?.textContent).toBe('second');
  });
});

describe('ordinary Markdown still renders', () => {
  it('headings, emphasis, lists, quotes and tables', () => {
    const { container } = show('## Head\n\n**bold** and *em* and `code`\n\n- a\n- b\n\n1. one\n\n> quote\n\n| h1 | h2 |\n|----|----|\n| c1 | c2 |');
    expect(screen.getByRole('heading', { name: 'Head' })).toBeTruthy();
    expect(container.querySelector('strong')?.textContent).toBe('bold');
    expect(container.querySelector('em')?.textContent).toBe('em');
    expect(container.querySelectorAll('ul li')).toHaveLength(2);
    expect(container.querySelectorAll('ol li')).toHaveLength(1);
    expect(container.querySelector('blockquote')?.textContent).toContain('quote');
    expect(container.querySelectorAll('table td')).toHaveLength(2);
  });

  it('decodes character references the lexer leaves as written', () => {
    const { container } = show('Tom &amp; Jerry &lt;3');
    expect(container.textContent).toBe('Tom & Jerry <3');
  });
});

describe('source scan', () => {
  it('no chat source uses innerHTML, dangerouslySetInnerHTML, marked.parse or the Renderer', () => {
    const roots = ['src/components/chat', 'src/pages/workbench', 'src/stores'];
    const files = roots.flatMap(root => readdirSync(root).filter(name => /\.(ts|tsx)$/.test(name)).map(name => join(root, name)));
    for (const file of files.filter(name => /chat|Chat|Markdown|Composer|Message|Session|Target|Disclosures|clipboard|catalog/.test(name))) {
      const source = readFileSync(file, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
      expect(source, file).not.toMatch(/dangerouslySetInnerHTML|innerHTML|outerHTML|insertAdjacentHTML|marked\.parse|new Renderer|\bparse\(/);
    }
  });
});

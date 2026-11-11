import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Lexer } from 'marked';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ModalHost } from '@dgos/ui';
import Markdown from '../src/components/chat/Markdown';
import type { ChatWebSource } from '../src/stores/chatStore';
import { renderToString } from 'katex';
import { diagramInputFailure, mathInputFailure, parseRichOutput, spreadsheetCell, tableData } from '../src/components/chat/richMarkdownSafety';
import { RichMarkdown } from '../src/components/chat/RichMarkdown';
import { renderDiagram } from '../src/components/chat/richDiagramSandbox';
import type { ReactNode } from 'react';

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
describe('trusted source citations', () => {
  const source: ChatWebSource = { id: 'host-source-1', url: 'https://evidence.example.invalid/article', title: 'Host source', excerpt: 'Retained evidence excerpt', retrieved_at: 1700000000, kind: 'search_result', truncated: true };

  it('opens only a retained source preview and still confirms an external open', async () => {
    render(<><Markdown source="Evidence [1](dgos-source:host-source-1)" sources={[source]} /><ModalHost /></>);
    fireEvent.click(screen.getByRole('button', { name: '1' }));
    const preview = await screen.findByRole('dialog', { name: 'Host source' });
    expect(within(preview).getByText(source.excerpt)).toBeTruthy();
    expect(opened).not.toHaveBeenCalled();
    fireEvent.click(within(preview).getByRole('button', { name: '打开原网页…' }));
    const confirmation = await screen.findByRole('dialog', { name: '打开外部链接？' });
    expect(opened).not.toHaveBeenCalled();
    fireEvent.click(within(confirmation).getByRole('button', { name: '打开' }));
    await waitFor(() => expect(opened).toHaveBeenCalledWith(source.url));
  });

  it('keeps unknown IDs inert and never upgrades an arbitrary URL into evidence', () => {
    const { container } = render(<><Markdown source="[unknown](dgos-source:invented) [ordinary](https://evidence.example.invalid/article)" sources={[source]} /><ModalHost /></>);
    expect(screen.queryByRole('button', { name: 'unknown' })).toBeNull();
    expect(container.textContent).toContain('来源不可用');
    expect(screen.getByRole('button', { name: 'ordinary' })).toBeTruthy();
    expect(screen.queryByText(source.excerpt)).toBeNull();
    expect(opened).not.toHaveBeenCalled();
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

describe('bounded rich Markdown', () => {
  it('recognizes block and inline math without changing escaped dollars or code', () => {
    const { container } = show('Inline $x^2$ and \\(y+1\\). Escaped \\$z$.\n\n$$\n\\frac{1}{2}\n$$\n\n`$code$`\n\n```js\nconst price = "$5";\n```');
    expect(container.querySelectorAll('[data-rich-kind="math"]')).toHaveLength(3);
    expect(container.textContent).toContain('Escaped $z$');
    expect(container.querySelector('p > code')?.textContent).toBe('$code$');
    expect(container.querySelector('pre code')?.textContent).toBe('const price = "$5";');
  });

  it('renders Mermaid only on request, rejecting unsupported types and active features', () => {
    show('```mermaid\nflowchart TD\nA[Start] --> B[Done]\n```');
    expect(screen.getByRole('button', { name: '渲染图解' })).toBeTruthy();
    expect(document.querySelector('iframe.chat-rich-sandbox')).toBeNull();
    expect(diagramInputFailure('sequenceDiagram\nparticipant A as Alice\nA->>B: Request\nB-->>A: Response')).toBeNull();
    expect(diagramInputFailure('flowchart LR\nA --> B')).toBeNull();
    expect(diagramInputFailure('sequenceDiagram\n  participant User\n  participant Reply\n  participant Host\n  User->>Reply: Preview received image\n  Reply->>Host: Request opaque resource\n  Host-->>Reply: Bounded image\n  Reply-->>User: Local preview')).toBeNull();
    expect(diagramInputFailure('sequenceDiagram\nparticipant Image\nactor User\nUser->>Image: Show image icon style config')).toBeNull();
    expect(diagramInputFailure('flowchart LR\nA[Image preview] --> B[Local image]')).toBeNull();
    expect(diagramInputFailure('flowchart LR; A-->B; click A "https://evil.invalid"')).toBe('unsupported');
    expect(diagramInputFailure('flowchart LR\nA-->B; style A fill:red')).toBe('unsupported');
    for (const source of ['pie\n"x": 1', 'flowchart LR\nclick A "https://evil.invalid"', 'flowchart LR\nA[<img src=x>]', '%%{init: {"securityLevel":"loose"}}%%\nflowchart LR\nA-->B', 'sequenceDiagram\nlink A: Docs@https://evil.invalid']) expect(diagramInputFailure(source)).toBe('unsupported');
    expect(diagramInputFailure(`flowchart LR\n${'A'.repeat(8000)}`)).toBe('limit');
    expect(diagramInputFailure(`flowchart LR\n${Array.from({ length: 101 }, (_, i) => `A${i}-->A${i + 1}`).join('\n')}`)).toBe('limit');
  });

  it('shows a reason and copies complete source when rich input cannot render', async () => {
    const original = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    try {
      const source = `flowchart LR\n${'A'.repeat(8001)}`;
      render(<RichMarkdown kind="diagram" source={source} />);
      expect(screen.getByText('内容超出渲染限额，仍可复制完整源码。')).toBeTruthy();
      fireEvent.click(screen.getByRole('button', { name: '复制源码' }));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith(source));
      expect(screen.queryByRole('button', { name: '渲染图解' })).toBeNull();
    } finally { if (original) Object.defineProperty(navigator, 'clipboard', original); else Reflect.deleteProperty(navigator, 'clipboard'); }
  });

  it('uses genuine KaTeX output and admits only inert MathML', () => {
    const markup = renderToString('\\frac{a^2}{b} + \\sqrt{x}', { output: 'mathml', throwOnError: true, trust: false, strict: 'error', maxExpand: 200, maxSize: 10 });
    const node = parseRichOutput(markup, 'math', 'math-test');
    expect(node.tag).toBe('math');
    expect(JSON.stringify(node)).toContain('mfrac');
    expect(JSON.stringify(node)).toContain('msqrt');
    expect(mathInputFailure('\\href{https://evil.invalid}{x}')).toBe('unsupported');
    expect(mathInputFailure('\\htmlStyle{position:fixed}{x}')).toBe('unsupported');
    expect(mathInputFailure('x'.repeat(4001))).toBe('limit');
    expect(() => renderToString('\\frac{', { throwOnError: true })).toThrow();
    expect(() => parseRichOutput('<math xmlns="http://www.w3.org/1998/Math/MathML"><mtext href="https://evil.invalid">x</mtext></math>', 'math', 'x')).toThrow('unsupported');
  });

  it('reconstructs SVG primitives and rejects script, HTML, image and external references', () => {
    const safe = parseRichOutput('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><style>text{fill:red}</style><defs><marker id="arrow"><path d="M0 0L1 1"/></marker></defs><path d="M0 0L20 20" marker-end="url(#arrow)"/><text x="5" y="5">Safe</text></svg>', 'diagram', 'render-1');
    expect(JSON.stringify(safe)).toContain('url(#render-1-arrow)');
    expect(JSON.stringify(safe)).not.toContain('fill:red');
    for (const child of ['<script>alert(1)</script>', '<foreignObject><div>html</div></foreignObject>', '<image href="https://evil.invalid/pixel"/>', '<a href="https://evil.invalid"><text>x</text></a>', '<path onload="alert(1)"/>', '<path marker-end="url(https://evil.invalid/x)"/>']) expect(() => parseRichOutput(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">${child}</svg>`, 'diagram', 'x')).toThrow('unsupported');
    expect(() => parseRichOutput('<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///etc/passwd">]><svg/>', 'diagram', 'x')).toThrow('unsupported');
  });

  it('discards Mermaid 12 default shadow filters and unused sequence icons without admitting effects or use links', () => {
    // Observed in genuine Mermaid 12.3.0 output for A[Start] --> B[Done] and A->>B: Request.
    const output = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><filter id="diagram-drop-shadow"><feDropShadow dx="4" dy="4" stdDeviation="0"/></filter><symbol id="diagram-computer"><path d="M2 2v13h20v-13z"/></symbol><marker id="diagram-arrowhead"><path d="M0 0L10 5L0 10z"/></marker></defs><path d="M0 0L20 20" marker-end="url(#diagram-arrowhead)"/><text>Start</text></svg>';
    const node = parseRichOutput(output, 'diagram', 'observed');
    expect(JSON.stringify(node)).toContain('Start');
    expect(JSON.stringify(node)).toContain('url(#observed-diagram-arrowhead)');
    expect(JSON.stringify(node)).not.toContain('filter');
    expect(JSON.stringify(node)).not.toContain('feDropShadow');
    expect(JSON.stringify(node)).not.toContain('symbol');
    expect(() => parseRichOutput('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><use href="#diagram-computer"/></svg>', 'diagram', 'x')).toThrow('unsupported');
  });

  it('keeps the normal chat source boundary when a native citation renderer is absent', () => {
    const citationRenderer = vi.fn((id: string, children: ReactNode) => <span data-native-id={id}>{children}</span>);
    const view = render(<Markdown source="> [native](dgos-source:retained)" citationRenderer={citationRenderer} />);
    expect(view.container.querySelector('[data-native-id="retained"]')?.textContent).toBe('native');
    view.rerender(<Markdown source="> [native](dgos-source:retained)" />);
    expect(view.container.querySelector('[data-native-id]')).toBeNull();
    expect(view.container.textContent).toContain('来源不可用');
  });

  it('isolates diagram execution, authenticates replies and removes aborted frames', async () => {
    const controller = new AbortController();
    const source = 'sequenceDiagram\nAlice->>Bob: A private source label';
    const result = renderDiagram(source, controller.signal);
    let frame: HTMLIFrameElement | null = null;
    await waitFor(() => { frame = document.querySelector('iframe.chat-rich-sandbox'); expect(frame).not.toBeNull(); });
    const isolated = frame as unknown as HTMLIFrameElement;
    expect(isolated.getAttribute('sandbox')).toBe('allow-scripts');
    expect(isolated.srcdoc).toContain("connect-src 'none'");
    expect(isolated.srcdoc).toContain("img-src 'none'");
    expect(isolated.srcdoc).toContain("default-src 'none'");
    expect(isolated.srcdoc).not.toContain('A private source label');
    const channel = /const channel = "([a-f0-9]+)"/.exec(isolated.srcdoc)?.[1];
    expect(channel).toBeTruthy();
    const reply = { channel, markup: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><text>safe</text></svg>' };
    let settled = false;
    void result.then(() => { settled = true; });
    window.dispatchEvent(new MessageEvent('message', { origin: 'null', source: window, data: { channel, type: 'ready' } }));
    window.dispatchEvent(new MessageEvent('message', { origin: 'null', source: isolated.contentWindow, data: reply }));
    await Promise.resolve();
    expect(settled).toBe(false);
    window.dispatchEvent(new MessageEvent('message', { origin: 'null', source: isolated.contentWindow, data: { channel, type: 'ready' } }));
    window.dispatchEvent(new MessageEvent('message', { origin: 'null', source: isolated.contentWindow, data: { ...reply, channel: 'wrong-channel' } }));
    await Promise.resolve();
    expect(settled).toBe(false);
    window.dispatchEvent(new MessageEvent('message', { origin: 'null', source: isolated.contentWindow, data: reply }));
    await expect(result).resolves.toBe(reply.markup);
    expect(isolated.isConnected).toBe(false);
    const abortedController = new AbortController();
    const aborted = renderDiagram('flowchart LR\nA-->B', abortedController.signal);
    const rejected = expect(aborted).rejects.toThrow('unavailable');
    await waitFor(() => expect(document.querySelector('iframe.chat-rich-sandbox')).not.toBeNull());
    abortedController.abort();
    await rejected;
    expect(document.querySelector('iframe.chat-rich-sandbox')).toBeNull();
  });

  it('displays genuine KaTeX MathML from the worker protocol and terminates the worker', async () => {
    const terminated = vi.fn();
    class FormulaWorker {
      onmessage: ((event: MessageEvent) => void) | null = null;
      onerror: (() => void) | null = null;
      terminate = terminated;
      postMessage(input: { token: string; source: string; display: boolean }) {
        const markup = renderToString(input.source, { displayMode: input.display, output: 'mathml', throwOnError: true, trust: false, strict: 'error', maxExpand: 200, maxSize: 10 });
        queueMicrotask(() => this.onmessage?.({ data: { token: input.token, markup } } as MessageEvent));
      }
    }
    vi.stubGlobal('Worker', FormulaWorker);
    try {
      const { container } = show('A formula $x^2$ next to `inline code`.');
      await waitFor(() => expect(container.querySelector('math msup')).not.toBeNull());
      expect(container.querySelector('math')?.textContent).toContain('x');
      expect(container.querySelector('p > code')?.textContent).toBe('inline code');
      expect(terminated).toHaveBeenCalledTimes(1);
      expect(container.querySelector('script, iframe, img, a')).toBeNull();
    } finally { cleanup(); vi.unstubAllGlobals(); }
  });

  it('times out and destroys a diagram frame that never replies', async () => {
    vi.useFakeTimers();
    try {
      const result = renderDiagram('flowchart LR\nA-->B', new AbortController().signal);
      const expired = expect(result).rejects.toThrow('timeout');
      await vi.advanceTimersByTimeAsync(5000);
      await expired;
      expect(document.querySelector('iframe.chat-rich-sandbox')).toBeNull();
    } finally { vi.useRealTimers(); }
  });
});

describe('on-demand data tables', () => {
  const largeTable = '| Name | Value |\n|---|---|\n' + Array.from({ length: 63 }, (_, i) => `| Item ${63 - i} | ${i === 62 ? '=SUM(1,2)' : i} |`).join('\n');

  it('mounts only a table page, sorts a view and restores the original order', () => {
    const { container } = show(largeTable);
    expect(container.querySelector('table')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: '显示表格' }));
    expect(container.querySelectorAll('tbody tr')).toHaveLength(50);
    expect(container.querySelector('tbody td')?.textContent).toBe('Item 63');
    fireEvent.click(screen.getByRole('button', { name: '按 Name 排序' }));
    expect(container.querySelector('tbody td')?.textContent).toBe('Item 1');
    expect(container.querySelector('th')?.getAttribute('aria-sort')).toBe('ascending');
    fireEvent.click(screen.getByRole('button', { name: '恢复原始顺序' }));
    expect(container.querySelector('tbody td')?.textContent).toBe('Item 63');
    fireEvent.click(screen.getByRole('button', { name: '下一组行' }));
    expect(container.querySelectorAll('tbody tr')).toHaveLength(13);
    fireEvent.click(screen.getByRole('button', { name: '收起表格' }));
    expect(container.querySelector('table')).toBeNull();
  });

  it('copies all original rows despite paging or sorting and escapes spreadsheet formulas', async () => {
    const original = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
    try {
      show(largeTable);
      fireEvent.click(screen.getByRole('button', { name: '显示表格' }));
      fireEvent.click(screen.getByRole('button', { name: '按 Name 排序' }));
      fireEvent.click(screen.getByRole('button', { name: '复制完整表格' }));
      await waitFor(() => expect(writeText).toHaveBeenCalled());
      const copied = writeText.mock.calls[0][0] as string;
      expect(copied.split('\r\n')).toHaveLength(64);
      expect(copied.indexOf('Item 63')).toBeLessThan(copied.indexOf('Item 1"'));
      expect(copied).toContain("'=SUM(1,2)");
    } finally { if (original) Object.defineProperty(navigator, 'clipboard', original); else Reflect.deleteProperty(navigator, 'clipboard'); }
  });

  it('exports complete CSV and reports export failures', async () => {
    const create = vi.fn().mockReturnValue('blob:table-export');
    const revoke = vi.fn();
    const createDescriptor = Object.getOwnPropertyDescriptor(URL, 'createObjectURL');
    const revokeDescriptor = Object.getOwnPropertyDescriptor(URL, 'revokeObjectURL');
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: create });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: revoke });
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => undefined);
    try {
      show(largeTable);
      fireEvent.click(screen.getByRole('button', { name: '导出完整 CSV' }));
      expect(create).toHaveBeenCalledTimes(1);
      expect(click).toHaveBeenCalledTimes(1);
      const csv = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(reader.error);
        reader.readAsText(create.mock.calls[0][0] as Blob);
      });
      expect(csv.split('\r\n')).toHaveLength(64);
      expect(csv).toContain("'=SUM(1,2)");
      expect(csv.indexOf('Item 63')).toBeLessThan(csv.indexOf('Item 1"'));
      expect(screen.getByText('已请求导出 CSV')).toBeTruthy();
      create.mockImplementation(() => { throw new Error('unavailable'); });
      fireEvent.click(screen.getByRole('button', { name: '导出完整 CSV' }));
      expect(screen.getByText('CSV 导出失败，请改用复制表格。')).toBeTruthy();
    } finally {
      if (createDescriptor) Object.defineProperty(URL, 'createObjectURL', createDescriptor); else Reflect.deleteProperty(URL, 'createObjectURL');
      if (revokeDescriptor) Object.defineProperty(URL, 'revokeObjectURL', revokeDescriptor); else Reflect.deleteProperty(URL, 'revokeObjectURL');
    }
  });

  it('quotes cells and neutralizes leading whitespace, control characters and formula prefixes', () => {
    for (const input of ['=1+1', '+CMD()', '-1', '@SUM(A1)', ' \t=1', '\ufeff=1', '\tvalue']) expect(spreadsheetCell(input)).toBe(`'${input}`);
    expect(spreadsheetCell('ordinary')).toBe('ordinary');
    expect(tableData([['a,b', '"quoted"', '=1']], ',')).toBe('"a,b","""quoted""","\'=1"');
  });
});

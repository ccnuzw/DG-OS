import { Component, Fragment, useMemo, useState, type ReactNode } from 'react';
import { Lexer, type Token, type Tokens } from 'marked';
import { common, createLowlight } from 'lowlight';
import { openUrl } from '@tauri-apps/plugin-opener';
import { Button, Modal, cn } from '@dgos/ui';
import { COPY_TEXT, useCopyFeedback } from './clipboard';
import './Markdown.css';

// Assistant Markdown (FR-006 AC14, AC29, B06-62; D077). Only the marked Lexer is used: its tokens are mapped to React
// elements here, so no HTML string is ever produced and nothing needs dangerouslySetInnerHTML.
//  - raw HTML tokens render as inert text;
//  - images never load: alt text and the URL are shown as text;
//  - only https: links are interactive, and opening one asks for confirmation first; every other protocol is text;
//  - any exception while lexing or rendering falls back to the plain text of the raw content.
//
// marked's `Token` union has an open `Generic` member, so switching on `type` does not narrow it. The lexer guarantees
// the shape that belongs to each `type`, which is why each case below names its token with a cast.

const FOCUS = 'focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-focus-ring';
const MAX_HIGHLIGHT_CHARS = 20000;
const lowlight = createLowlight(common);

const NAMED_ENTITIES: Readonly<Record<string, string>> = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: '\u00a0' };

/** The lexer keeps character references as written; a browser would show their characters, so decode the common ones. */
function decodeEntities(text: string): string {
  return text.replace(/&(?:#(\d{1,7})|#[xX]([0-9a-fA-F]{1,6})|([a-zA-Z]+));/g, (whole, dec?: string, hex?: string, name?: string) => {
    if (name) return Object.prototype.hasOwnProperty.call(NAMED_ENTITIES, name) ? NAMED_ENTITIES[name] : whole;
    const code = dec ? Number(dec) : parseInt(hex ?? '', 16);
    if (!Number.isInteger(code) || code <= 0 || code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)) return whole;
    return String.fromCodePoint(code);
  });
}

/** The normalized URL when it is a plain https link, else null. */
function httpsTarget(href: string): string | null {
  try {
    const url = new URL(href);
    return url.protocol === 'https:' && url.hostname ? url.href : null;
  } catch {
    return null;
  }
}

function LinkButton({ url, children }: { url: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button variant="link" className={cn('inline h-auto whitespace-normal text-left underline wrap-anywhere', FOCUS)} onClick={() => setOpen(true)}>
        {children}
      </Button>
      <Modal
        open={open}
        onOpenChange={setOpen}
        title="打开外部链接？"
        icon="warning"
        description={<span>将使用系统浏览器打开此地址，请确认来源可信：<span className="mt-2 block font-mono text-xs wrap-anywhere">{url}</span></span>}
        confirmText="打开"
        onConfirm={() => openUrl(url)}
      />
    </>
  );
}

interface HastText { type: 'text'; value: string }
interface HastElement { type: 'element'; properties?: { className?: unknown }; children: unknown[] }

function isHastText(node: unknown): node is HastText {
  return typeof node === 'object' && node !== null && 'type' in node && node.type === 'text' && 'value' in node && typeof node.value === 'string';
}
function isHastElement(node: unknown): node is HastElement {
  return typeof node === 'object' && node !== null && 'type' in node && node.type === 'element' && 'children' in node && Array.isArray(node.children);
}

/** Only the `hljs-*` classes lowlight emits survive; everything else about a node is ignored. */
function hastToReact(nodes: readonly unknown[]): ReactNode[] {
  return nodes.map((node, index) => {
    if (isHastText(node)) return <Fragment key={index}>{node.value}</Fragment>;
    if (!isHastElement(node)) return null;
    const props = node.properties;
    const raw = typeof props === 'object' && props !== null ? props.className : undefined;
    const classes = (Array.isArray(raw) ? raw : []).filter((name): name is string => typeof name === 'string' && /^hljs-[a-z_-]+$/.test(name));
    return <span key={index} className={classes.join(' ') || undefined}>{hastToReact(node.children)}</span>;
  });
}

function highlight(lang: string | undefined, text: string): ReactNode[] | null {
  const name = lang?.trim().split(/\s+/)[0]?.toLowerCase();
  if (!name || text.length > MAX_HIGHLIGHT_CHARS || !lowlight.registered(name)) return null;
  try {
    return hastToReact(lowlight.highlight(name, text).children);
  } catch {
    return null;
  }
}

function CodeBlock({ lang, text }: { lang?: string; text: string }) {
  const [state, copy] = useCopyFeedback();
  const label = lang?.trim().split(/\s+/)[0] ?? '';
  const highlighted = useMemo(() => highlight(label, text), [label, text]);
  return (
    <div className="my-2 overflow-hidden rounded-md border border-border bg-secondary">
      <div className="flex items-center justify-between gap-2 border-b border-border px-3 py-1 text-xs text-muted-foreground">
        <span className="min-w-0 wrap-anywhere">{label || '代码'}</span>
        <span className="flex items-center gap-2">
          <span role="status" className={cn('text-xs', state === 'failed' ? 'text-error-text' : 'text-success-text')}>{COPY_TEXT[state]}</span>
          <Button size="sm" variant="ghost" onClick={() => copy(text)}>复制代码</Button>
        </span>
      </div>
      <pre tabIndex={0} aria-label={label ? `${label} 代码` : '代码'} className={cn('chat-code m-0 overflow-x-auto p-3 font-mono text-xs leading-relaxed', FOCUS)}>
        <code>{highlighted ?? text}</code>
      </pre>
    </div>
  );
}

const HEADING_CLASS: Readonly<Record<number, string>> = {
  1: 'text-lg', 2: 'text-base', 3: 'text-base', 4: 'text-sm', 5: 'text-sm', 6: 'text-sm',
};
const ALIGN_CLASS: Readonly<Record<string, string>> = { left: 'text-left', center: 'text-center', right: 'text-right' };

/** Child tokens of a token type this renderer does not know, if it has any. Unknown types fall back to their raw text. */
function childTokens(token: Token): Token[] | undefined {
  return 'tokens' in token && Array.isArray(token.tokens) && token.tokens.length ? token.tokens : undefined;
}

function renderInline(tokens: readonly Token[] | undefined): ReactNode[] {
  return (tokens ?? []).map((token, index): ReactNode => {
    switch (token.type) {
      case 'text': {
        const text = token as Tokens.Text;
        return <Fragment key={index}>{text.tokens?.length ? renderInline(text.tokens) : decodeEntities(text.text)}</Fragment>;
      }
      case 'escape': {
        const escape = token as Tokens.Escape;
        return <Fragment key={index}>{escape.text}</Fragment>;
      }
      case 'strong': {
        const strong = token as Tokens.Strong;
        return <strong key={index}>{renderInline(strong.tokens)}</strong>;
      }
      case 'em': {
        const em = token as Tokens.Em;
        return <em key={index}>{renderInline(em.tokens)}</em>;
      }
      case 'del': {
        const del = token as Tokens.Del;
        return <del key={index}>{renderInline(del.tokens)}</del>;
      }
      case 'codespan': {
        const codespan = token as Tokens.Codespan;
        return <code key={index} className="rounded-sm bg-secondary px-1 font-mono text-xs">{codespan.text}</code>;
      }
      case 'br':
        return <br key={index} />;
      case 'checkbox': {
        const checkbox = token as Tokens.Checkbox;
        return <input key={index} type="checkbox" disabled checked={checkbox.checked} readOnly aria-label="任务状态" className="mr-1 align-middle" />;
      }
      case 'link': {
        const link = token as Tokens.Link;
        const target = httpsTarget(link.href);
        if (!link.autolink && target) return <LinkButton key={index} url={target}>{renderInline(link.tokens)}</LinkButton>;
        // Not interactive: other protocols, and autolinks, which carry no text the user chose to click.
        return <span key={index}>{renderInline(link.tokens)} ({link.href})</span>;
      }
      case 'image': {
        const image = token as Tokens.Image;
        return <span key={index}>[图片：{image.text || '无说明'}] {image.href}</span>;
      }
      case 'html':
        return <span key={index}>{token.raw}</span>;
      default:
        return <Fragment key={index}>{childTokens(token) ? renderInline(childTokens(token)) : decodeEntities(token.raw)}</Fragment>;
    }
  });
}

function renderBlocks(tokens: readonly Token[]): ReactNode[] {
  return tokens.map((token, index): ReactNode => {
    switch (token.type) {
      case 'space':
      case 'def':
        return null;
      case 'heading': {
        const heading = token as Tokens.Heading;
        return <p key={index} role="heading" aria-level={Math.min(Math.max(heading.depth + 2, 3), 6)}
          className={cn('m-0 mt-3 mb-1 font-semibold', HEADING_CLASS[heading.depth] ?? 'text-sm')}>{renderInline(heading.tokens)}</p>;
      }
      case 'paragraph': {
        const paragraph = token as Tokens.Paragraph;
        return <p key={index} className="m-0 my-2 wrap-anywhere">{renderInline(paragraph.tokens)}</p>;
      }
      case 'text': {
        const text = token as Tokens.Text;
        return <p key={index} className="m-0 wrap-anywhere">{text.tokens?.length ? renderInline(text.tokens) : decodeEntities(text.text)}</p>;
      }
      case 'code': {
        const code = token as Tokens.Code;
        return <CodeBlock key={index} lang={code.lang} text={code.text} />;
      }
      case 'blockquote': {
        const quote = token as Tokens.Blockquote;
        return <blockquote key={index} className="m-0 my-2 border-l-4 border-border pl-3 text-muted-foreground">{renderBlocks(quote.tokens)}</blockquote>;
      }
      case 'list': {
        const list = token as Tokens.List;
        const items = list.items.map((item, itemIndex) => <li key={itemIndex} className="my-1 wrap-anywhere">{renderBlocks(item.tokens)}</li>);
        return list.ordered
          ? <ol key={index} start={typeof list.start === 'number' ? list.start : undefined} className="m-0 my-2 list-decimal pl-6">{items}</ol>
          : <ul key={index} className="m-0 my-2 list-disc pl-6">{items}</ul>;
      }
      case 'table': {
        const table = token as Tokens.Table;
        const cell = (c: Tokens.TableCell, key: number, header: boolean) => {
          const Tag = header ? 'th' : 'td';
          return <Tag key={key} className={cn('border border-border px-2 py-1', header && 'bg-secondary font-semibold', c.align ? ALIGN_CLASS[c.align] : 'text-left')}>{renderInline(c.tokens)}</Tag>;
        };
        return (
          <div key={index} className="my-2 overflow-x-auto">
            <table className="border-collapse text-sm">
              <thead><tr>{table.header.map((c, i) => cell(c, i, true))}</tr></thead>
              <tbody>{table.rows.map((row, r) => <tr key={r}>{row.map((c, i) => cell(c, i, false))}</tr>)}</tbody>
            </table>
          </div>
        );
      }
      case 'hr':
        return <hr key={index} className="my-3 border-border" />;
      case 'html':
        // Raw HTML is shown as the characters the model wrote, never interpreted.
        return <p key={index} className="m-0 my-2 whitespace-pre-wrap font-mono text-xs wrap-anywhere">{token.raw.trimEnd()}</p>;
      default:
        return <p key={index} className="m-0 wrap-anywhere">{childTokens(token) ? renderInline(childTokens(token)) : decodeEntities(token.raw)}</p>;
    }
  });
}

function PlainText({ source }: { source: string }) {
  return <pre className="m-0 whitespace-pre-wrap font-sans text-sm wrap-anywhere">{source}</pre>;
}

function MarkdownBody({ source }: { source: string }) {
  const tokens = useMemo(() => Lexer.lex(source), [source]);
  return <div className="text-sm leading-relaxed">{renderBlocks(tokens)}</div>;
}

interface BoundaryProps { source: string; children: ReactNode }
interface BoundaryState { failed: boolean; source: string }

/** Catches a lexer or render exception and shows the raw content as plain text instead (AC29). */
class MarkdownBoundary extends Component<BoundaryProps, BoundaryState> {
  state: BoundaryState = { failed: false, source: this.props.source };

  static getDerivedStateFromError(): Partial<BoundaryState> {
    return { failed: true };
  }

  static getDerivedStateFromProps(props: BoundaryProps, state: BoundaryState): BoundaryState | null {
    return props.source === state.source ? null : { failed: false, source: props.source };
  }

  render() {
    return this.state.failed ? <PlainText source={this.props.source} /> : this.props.children;
  }
}

export default function Markdown({ source }: { source: string }) {
  return <MarkdownBoundary source={source}><MarkdownBody source={source} /></MarkdownBoundary>;
}

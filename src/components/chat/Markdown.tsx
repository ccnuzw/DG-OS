import { t, useLocale as usePageLocale } from '../../i18n';
import { Component, Fragment, createContext, useContext, useId, useMemo, useState, type ReactNode } from 'react';
import { Lexer, type Token, type Tokens } from 'marked';
import { common, createLowlight } from 'lowlight';
import { Button, cn } from '@dgos/ui';
import { ChevronDown, ChevronUp, Copy, FileCode2 } from 'lucide-react';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { COPY_TEXT, useCopyFeedback } from './clipboard';
import type { ChatWebSource } from '../../stores/chatStore';
import { ConfirmedExternalLink, SourceCitation, TrustedSourcesContext } from './SourcePreview';
import { RichMarkdown } from './RichMarkdown';
import { MarkdownTable } from './MarkdownTable';
import { richMarkdownOptions, type MathToken } from './richMarkdownTokens';
import './Markdown.css';

// Assistant Markdown (FR-006 AC14, AC29, B06-62; D077). marked tokens become React elements; raw HTML is never
// interpreted. Rich output is rendered in isolation and rebuilt from allowlisted SVG/MathML primitives.
//  - raw HTML tokens render as inert text;
//  - images never load: alt text and the URL are shown as text;
//  - only https: links are interactive, and opening one asks for confirmation first; every other protocol is text;
//  - any exception while lexing or rendering falls back to the plain text of the raw content.
//
// marked's `Token` union has an open `Generic` member, so switching on `type` does not narrow it. The lexer guarantees
// the shape that belongs to each `type`, which is why each case below names its token with a cast.

const MAX_HIGHLIGHT_CHARS = 20000;
const lowlight = createLowlight(common);

export const MarkdownHighlightContext = createContext(true);
const CitationRendererContext = createContext<((id: string, children: ReactNode) => ReactNode) | undefined>(undefined);

function MarkdownCitation({ id, children }: { id: string; children: ReactNode }) {
  const renderCitation = useContext(CitationRendererContext);
  return renderCitation ? renderCitation(id, children) : <SourceCitation id={id}>{children}</SourceCitation>;
}

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

export function CodeBlock({ lang, text }: { lang?: string; text: string }) {
  usePageLocale();
  const codeHighlighting = useContext(MarkdownHighlightContext);
  const [state, copy] = useCopyFeedback();
  const [expanded, setExpanded] = useState(false);
  const codeId = useId();
  const lines = useMemo(() => text.split('\n'), [text]);
  const long = lines.length > 24;
  const collapsed = long && !expanded;
  const visibleText = collapsed ? lines.slice(0, 12).join('\n') : text;
  const label = lang?.trim().split(/\s+/)[0] ?? '';
  const highlighted = useMemo(() => codeHighlighting ? highlight(label, visibleText) : null, [label, visibleText, codeHighlighting]);
  return (
    <div className="chat-code-block">
      <div className="chat-code-toolbar">
        <span className="chat-code-label"><span className="chat-code-icon"><FileCode2 size={15} aria-hidden="true" /></span><strong className="chat-code-language">{label || t('chat.code')}</strong><small className="chat-code-count">{t('agentExecution.codeLines', { count: lines.length })}</small></span>
        <span className="chat-code-actions">
          <span role="status" className={cn('text-xs', state === 'failed' ? 'text-status-error-text' : 'text-status-success-text')}>{COPY_TEXT[state]}</span>
          <Button size="sm" variant="ghost" onClick={() => copy(text)}><Copy size={13} aria-hidden="true" />{t('chat.copyCode')}</Button>
        </span>
      </div>
      <pre id={codeId} tabIndex={0} aria-label={label ? t('chat.codeLabel', { language: label }) : t('chat.code')} className={cn('chat-code type-code m-0 overflow-x-auto p-3', FOCUS_RING)} data-code-collapsed={collapsed || undefined}>
        <code>{highlighted ?? visibleText}</code>
      </pre>
      {long ? <Button className="chat-code-toggle" size="sm" variant="ghost" aria-expanded={expanded} aria-controls={codeId} onClick={() => setExpanded(value => !value)}>{expanded ? <ChevronUp size={14} aria-hidden="true" /> : <ChevronDown size={14} aria-hidden="true" />}{t(expanded ? 'agentExecution.collapseCode' : 'agentExecution.expandCode', { count: lines.length })}</Button> : null}
    </div>
  );
}

const HEADING_CLASS: Readonly<Record<number, string>> = {
  1: 'chat-markdown-heading-large', 2: 'chat-markdown-heading-medium', 3: 'chat-markdown-heading-medium', 4: 'chat-markdown-heading-small', 5: 'chat-markdown-heading-small', 6: 'chat-markdown-heading-small',
};

/** Child tokens of a token type this renderer does not know, if it has any. Unknown types fall back to their raw text. */
function childTokens(token: Token): Token[] | undefined {
  return 'tokens' in token && Array.isArray(token.tokens) && token.tokens.length ? token.tokens : undefined;
}

function renderInline(tokens: readonly Token[] | undefined): ReactNode[] {
  return (tokens ?? []).map((token, index): ReactNode => {
    switch (token.type) {
      case 'richMath': {
        const math = token as MathToken;
        return <RichMarkdown key={`${index}:${math.raw}`} kind="math" source={math.text} display={math.display} />;
      }
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
        return <code key={index} className="type-code rounded-sm px-1">{codespan.text}</code>;
      }
      case 'br':
        return <br key={index} />;
      case 'checkbox': {
        const checkbox = token as Tokens.Checkbox;
        return <input key={index} type="checkbox" disabled checked={checkbox.checked} readOnly aria-label={t('chat.taskStatus')} className="mr-1 align-middle" />;
      }
      case 'link': {
        const link = token as Tokens.Link;
        if (!link.autolink && link.href.startsWith('dgos-source:')) return <MarkdownCitation key={index} id={link.href.slice('dgos-source:'.length)}>{renderInline(link.tokens)}</MarkdownCitation>;
        const target = httpsTarget(link.href);
        if (!link.autolink && target) return <ConfirmedExternalLink key={index} url={target}>{renderInline(link.tokens)}</ConfirmedExternalLink>;
        // Not interactive: other protocols, and autolinks, which carry no text the user chose to click.
        return <span key={index}>{renderInline(link.tokens)} ({link.href})</span>;
      }
      case 'image': {
        const image = token as Tokens.Image;
        return <span key={index}>{t('chat.image')}{image.text || t('chat.noAlt')}] {image.href}</span>;
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
      case 'richMath': {
        const math = token as MathToken;
        return <RichMarkdown key={`${index}:${math.raw}`} kind="math" source={math.text} display={math.display} />;
      }
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
        const language = code.lang?.trim().split(/\s+/)[0]?.toLowerCase();
        if (language === 'mermaid') return <RichMarkdown key={`${index}:${code.text}`} kind="diagram" source={code.text} />;
        if (language === 'math' || language === 'latex' || language === 'tex') return <RichMarkdown key={`${index}:${code.text}`} kind="math" source={code.text} />;
        return <CodeBlock key={index} lang={code.lang} text={code.text} />;
      }
      case 'blockquote': {
        const quote = token as Tokens.Blockquote;
        return <blockquote key={index} className="m-0 my-2 border-l-4 border-border-subtle pl-3 text-ink-secondary">{renderBlocks(quote.tokens)}</blockquote>;
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
        return <MarkdownTable key={index} table={table} renderCell={cell => renderInline(cell.tokens)} />;
      }
      case 'hr':
        return <hr key={index} className="my-3 border-border-subtle" />;
      case 'html':
        // Raw HTML is shown as the characters the model wrote, never interpreted.
        return <p key={index} className="m-0 my-2 whitespace-pre-wrap font-mono text-xs wrap-anywhere">{token.raw.trimEnd()}</p>;
      default:
        return <p key={index} className="m-0 wrap-anywhere">{childTokens(token) ? renderInline(childTokens(token)) : decodeEntities(token.raw)}</p>;
    }
  });
}

function PlainText({ source }: { source: string }) {
  usePageLocale();
  return <pre className="type-reading m-0 whitespace-pre-wrap wrap-anywhere">{source}</pre>;
}

function MarkdownBody({ source }: { source: string }) {
  usePageLocale();
  const tokens = useMemo(() => Lexer.lex(source, richMarkdownOptions), [source]);
  return <div className="type-reading chat-markdown">{renderBlocks(tokens)}</div>;
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

export default function Markdown({ source, sources = [], citationRenderer }: { source: string; sources?: ChatWebSource[]; citationRenderer?: (id: string, children: ReactNode) => ReactNode }) {
  usePageLocale();
  return <TrustedSourcesContext.Provider value={sources}><CitationRendererContext.Provider value={citationRenderer}><MarkdownBoundary source={source}><MarkdownBody source={source} /></MarkdownBoundary></CitationRendererContext.Provider></TrustedSourcesContext.Provider>;
}

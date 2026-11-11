import { createElement, useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { Button, cn } from '@dgos/ui';
import { ChevronDown, Code, Copy, Info, Network, Play, ShieldCheck, Sigma, TriangleAlert } from 'lucide-react';
import { t, useLocale } from '../../i18n';
import { COPY_TEXT, useCopyFeedback } from './clipboard';
import { diagramInputFailure, mathInputFailure, parseRichOutput, MAX_RICH_OUTPUT_CHARS, RICH_RENDER_TIMEOUT_MS, type RichRenderFailure, type SafeRichNode } from './richMarkdownSafety';
import { renderDiagram } from './richDiagramSandbox';
import './RichMarkdown.css';

const REACT_ATTRIBUTE: Readonly<Record<string, string>> = { class: 'className', 'text-anchor': 'textAnchor', 'dominant-baseline': 'dominantBaseline', 'alignment-baseline': 'alignmentBaseline', 'font-size': 'fontSize', 'font-weight': 'fontWeight', 'stroke-width': 'strokeWidth', 'stroke-dasharray': 'strokeDasharray', 'marker-start': 'markerStart', 'marker-mid': 'markerMid', 'marker-end': 'markerEnd' };
function safeNode(node: SafeRichNode | string, index = 0): ReactNode {
  if (typeof node === 'string') return node;
  const props: Record<string, unknown> = { key: index };
  for (const [name, value] of Object.entries(node.attributes)) props[REACT_ATTRIBUTE[name] ?? name] = value;
  if (node.tag === 'svg') {
    // viewBox geometry has already passed parseRichOutput's finite dimension limits.
    const dimensions = node.attributes.viewBox.trim().split(/[\s,]+/).map(Number);
    props.width = `${dimensions[2] / 16}em`;
    props.height = `${dimensions[3] / 16}em`;
  }
  return createElement(node.tag, props, ...node.children.map((child, index) => safeNode(child, index)));
}

// Only one heavy renderer runs at a time across Markdown consumers. Unmounted queued work is skipped.
let pending: Promise<unknown> = Promise.resolve();
function renderQueued(kind: 'diagram' | 'math', source: string, display: boolean, signal: AbortSignal): Promise<string> {
  const result = pending.catch(() => undefined).then(async () => {
    if (signal.aborted) throw new Error('unavailable');
    if (kind === 'diagram') return renderDiagram(source, signal);
    if (typeof Worker === 'undefined') throw new Error('unavailable');
    // ES2020 WebViews do not expose Promise.withResolvers; retain the compatible executor form.
    return new Promise<string>((resolve, reject) => {
      const worker = new Worker(new URL('./richMath.worker.ts', import.meta.url), { type: 'module' });
      const token = crypto.randomUUID();
      let active = true;
      const finish = (failure?: RichRenderFailure, markup?: string) => {
        if (!active) return;
        active = false;
        clearTimeout(timer);
        signal.removeEventListener('abort', abort);
        worker.terminate();
        if (failure) reject(new Error(failure)); else resolve(markup!);
      };
      const abort = () => finish('unavailable');
      const timer = window.setTimeout(() => finish('timeout'), RICH_RENDER_TIMEOUT_MS);
      worker.onmessage = (event: MessageEvent<unknown>) => {
        if (!active || !event.data || typeof event.data !== 'object') return;
        const data = event.data as { token?: unknown; markup?: unknown; failure?: unknown };
        if (data.token !== token) return;
        if (typeof data.markup === 'string') finish(data.markup.length > MAX_RICH_OUTPUT_CHARS ? 'limit' : undefined, data.markup);
        else finish(data.failure === 'limit' || data.failure === 'unsupported' ? data.failure : 'failed');
      };
      worker.onerror = () => finish('failed');
      signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) { abort(); return; }
      worker.postMessage({ source, display, token });
    });
  });
  pending = result;
  return result;
}

export function RichMarkdownBoundary({ kind }: { kind: 'diagram' | 'math' | 'table' }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const Icon = kind === 'table' ? Info : ShieldCheck;
  return <span className="chat-rich-boundary-disclosure">
    <Button className="chat-rich-boundary-toggle" size="xs" variant="ghost" aria-expanded={open} aria-controls={id} onClick={() => setOpen(value => !value)}>
      <Icon size={13} aria-hidden="true" />{t(kind === 'table' ? 'richMarkdown.tableDetails' : 'richMarkdown.renderingDetails')}<ChevronDown size={12} className={cn('chat-rich-disclosure-chevron', open && 'chat-rich-disclosure-open')} aria-hidden="true" />
    </Button>
    {open ? <span id={id} className="chat-rich-boundary">{t(`richMarkdown.${kind}Boundary`)}</span> : null}
  </span>;
}

export function RichMarkdown({ kind, source, display = true }: { kind: 'diagram' | 'math'; source: string; display?: boolean }) {
  useLocale();
  const [copyState, copy] = useCopyFeedback();
  const [requested, setRequested] = useState(false);
  const [result, setResult] = useState<{ source: string; node?: SafeRichNode; failure?: RichRenderFailure } | null>(null);
  const [sourceChoice, setSourceChoice] = useState<{ source: string; open: boolean } | null>(null);
  const root = useRef<HTMLSpanElement>(null);
  const generation = useRef(0);
  const prefix = useId().replace(/[^A-Za-z0-9-]/g, '');
  const failure = useMemo(() => kind === 'diagram' ? diagramInputFailure(source) : mathInputFailure(source), [kind, source]);
  // Formulas load as they enter the viewport; diagrams require an explicit user action.
  useEffect(() => {
    if (kind !== 'math' || failure) return;
    if (typeof IntersectionObserver === 'undefined') { setRequested(true); return; }
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { setRequested(true); observer.disconnect(); } }, { rootMargin: '160px' });
    if (root.current) observer.observe(root.current);
    return () => observer.disconnect();
  }, [kind, source, failure]);
  useEffect(() => {
    if (!requested || failure) return;
    const current = ++generation.current;
    const controller = new AbortController();
    void renderQueued(kind, source, display, controller.signal).then(markup => {
      if (controller.signal.aborted || generation.current !== current) return;
      try { setResult({ source, node: parseRichOutput(markup, kind, prefix) }); }
      catch (error) { const message = error instanceof Error ? error.message : 'failed'; setResult({ source, failure: message === 'limit' || message === 'unsupported' ? message : 'failed' }); }
    }).catch(error => {
      if (controller.signal.aborted || generation.current !== current) return;
      const message = error instanceof Error ? error.message : 'failed';
      setResult({ source, failure: message === 'limit' || message === 'unsupported' || message === 'timeout' || message === 'unavailable' ? message : 'failed' });
    });
    return () => { generation.current++; controller.abort(); };
  }, [requested, kind, source, display, failure, prefix]);
  const current = result?.source === source ? result : null;
  const error = failure ?? current?.failure;
  const inline = kind === 'math' && !display;
  const sourceOpen = Boolean(error) || (sourceChoice?.source === source ? sourceChoice.open : kind === 'diagram' && !current?.node);
  const KindIcon = kind === 'diagram' ? Network : Sigma;
  const controls = <span className="chat-rich-controls">
    {!requested && !error ? <Button size="xs" variant="secondary" className="chat-rich-render" onClick={() => setRequested(true)}><Play size={13} aria-hidden="true" />{t(kind === 'diagram' ? 'richMarkdown.renderDiagram' : 'richMarkdown.renderFormula')}</Button> : null}
    <Button size="xs" variant="ghost" className="chat-rich-source-toggle" disabled={Boolean(error)} aria-expanded={sourceOpen} aria-controls={`${prefix}-source`} onClick={() => setSourceChoice({ source, open: !sourceOpen })}><Code size={13} aria-hidden="true" />{t('richMarkdown.source')}<ChevronDown size={12} className={cn('chat-rich-disclosure-chevron', sourceOpen && 'chat-rich-disclosure-open')} aria-hidden="true" /></Button>
    <Button size="xs" variant="ghost" onClick={() => copy(source)}><Copy size={13} aria-hidden="true" />{t('richMarkdown.copySource')}</Button>
    <span role="status" className={cn('chat-rich-copy-status', copyState === 'failed' ? 'text-status-error-text' : 'text-status-success-text')}>{COPY_TEXT[copyState]}</span>
  </span>;
  return <span ref={root} className={cn('chat-rich', inline ? 'chat-rich-inline' : 'chat-rich-block')} data-rich-kind={kind} data-rich-state={error ? 'error' : current?.node ? 'ready' : requested ? 'loading' : 'idle'} aria-busy={requested && !current && !error || undefined}>
    {!inline ? <span className="chat-rich-toolbar"><span className="chat-rich-heading"><span className="chat-rich-kind-icon"><KindIcon size={15} aria-hidden="true" /></span><strong>{t(kind === 'diagram' ? 'richMarkdown.diagram' : 'richMarkdown.formula')}</strong></span>{controls}</span> : null}
    {current?.node ? <span className={cn('chat-rich-output', kind === 'diagram' && 'chat-rich-diagram')} role={kind === 'diagram' ? 'img' : undefined} aria-label={kind === 'diagram' ? t('richMarkdown.diagram') : undefined} tabIndex={kind === 'diagram' ? 0 : undefined}>{safeNode(current.node)}</span> : null}
    {error ? <span className="chat-rich-feedback chat-rich-error" role="status"><TriangleAlert size={14} aria-hidden="true" /><span>{t(`richMarkdown.${error}`)}</span></span> : requested && !current ? <span className="chat-rich-feedback" role="status">{t('richMarkdown.loading')}</span> : null}
    {inline ? controls : null}
    {sourceOpen ? <span id={`${prefix}-source`} className="chat-rich-source" tabIndex={0}>{source}</span> : null}
    {!inline ? <RichMarkdownBoundary kind={kind} /> : null}
  </span>;
}

import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Button, Modal } from '@dgos/ui';
import { ChevronDown, ChevronRight, ExternalLink, Eye, EyeOff, FileImage, FileJson, FileText, FolderOpen, Globe, Info, Link, Search } from 'lucide-react';
import { formatDateTime, useLocale } from '../../i18n';
import type { AgentEvent, Json } from '../../stores/agentRunnerStore';
import { ConfirmedExternalLink } from '../chat/SourcePreview';
import { safeSourceUrl } from '../chat/sources';
import type { AgentReferenceActions } from './eventOutputReferences';
import { agentReplySources, boundedResourceFields, decodeNativeImage, NativeImageError, nativeResourcePath, nativeResources, NATIVE_RESOURCE_LIMITS, type AgentReplySource, type NativeImageFailure, type NativeResource } from './nativeReplyResourceData';
import './agent-reply-resources.css';

export type { AgentReplySource } from './nativeReplyResourceData';
const AgentReplySourcesContext = createContext<AgentReplySource[]>([]);

export function AgentReplySourcesProvider({ events, children }: { events: readonly AgentEvent[]; children: ReactNode }) {
  const sources = useMemo(() => agentReplySources(events), [events]);
  return <AgentReplySourcesContext.Provider value={sources}>{children}</AgentReplySourcesContext.Provider>;
}
export function useAgentReplySources(): AgentReplySource[] { return useContext(AgentReplySourcesContext); }

export function AgentReplySourcePreview({ source, children, className = '' }: { source: AgentReplySource; children?: ReactNode; className?: string }) {
  return <ReplySourcePreview key={JSON.stringify([source.id, source.url, source.eventSeq])} source={source} className={className}>{children}</ReplySourcePreview>;
}

function ReplySourcePreview({ source, children, className }: { source: AgentReplySource; children?: ReactNode; className: string }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const url = safeSourceUrl(source.url);
  return <>
    <Button type="button" variant="link" className={`agent-resource-source-link ${className}`} onClick={() => setOpen(true)}>{children ?? source.title}</Button>
    <Modal open={open} onOpenChange={setOpen} kind="alert" title={source.title} description={url ? new URL(url).hostname : t('agentResources.invalidUrl')} footer={<Button variant="secondary" onClick={() => setOpen(false)}>{t('agentResources.close')}</Button>}>
      <section className="agent-resource-source-preview">
        <div className="agent-resource-source-preview-meta"><span className="agent-resource-kind"><Globe size={12} aria-hidden="true" />{t(source.kind === 'web_page' ? 'agentResources.webPage' : 'agentResources.searchResult')}</span><span className="agent-resource-note">{source.retrievedAt === undefined ? t('agentResources.retrievedUnknown') : t('agentResources.retrievedAt', { time: formatDateTime(new Date(source.retrievedAt * 1000)) })}</span></div>
        <p className="agent-resource-text agent-resource-source-excerpt">{source.excerpt || t('agentResources.noExcerpt')}</p>
        {source.truncated ? <p className="agent-resource-warning">{t('agentResources.sourceTruncated')}</p> : null}
        <footer className="agent-resource-source-preview-footer">
          <p className="agent-resource-note agent-resource-boundary"><Info size={13} aria-hidden="true" /><span>{t('agentResources.sourceBoundary')}</span></p>
          {url ? <ConfirmedExternalLink url={url}><ExternalLink size={14} aria-hidden="true" />{t('agentResources.openExternal')}</ConfirmedExternalLink> : null}
          <p className="agent-resource-note agent-resource-provenance">{t('agentResources.sourceEvent', { seq: source.eventSeq })}</p>
        </footer>
      </section>
    </Modal>
  </>;
}

/** Missing or ambiguous native IDs remain visibly unresolved. */
export function AgentReplyCitation({ id, children }: { id: string; children: ReactNode }) {
  const { t } = useLocale();
  const matches = useAgentReplySources().filter(source => source.id === id);
  return matches.length === 1 ? <AgentReplySourcePreview source={matches[0]}>{children}</AgentReplySourcePreview> : <span className="agent-resource-citation-unavailable" title={t('agentResources.citationUnavailable')}>{children} ({t('agentResources.citationUnavailable')})</span>;
}

export function AgentReplySourceList({ events }: { events: readonly AgentEvent[] }) {
  const { t } = useLocale();
  const sources = useMemo(() => agentReplySources(events), [events]);
  if (!sources.length) return null;
  return <section className="agent-resource-sources" aria-label={t('agentResources.sources')}>
    <header><span className="agent-resource-heading-icon"><Search size={15} aria-hidden="true" /></span><strong>{t('agentResources.sources')}</strong><span className="agent-resource-count">{sources.length}</span></header>
    <ul>{sources.map(source => <li key={`${source.id ?? ''}:${source.url}`}>
      <AgentReplySourcePreview source={source} className="agent-resource-source-card">
        <span className="agent-resource-source-icon"><Globe size={16} aria-hidden="true" /></span>
        <span className="agent-resource-source-copy"><span className="agent-resource-source-title">{source.title}</span><span className="agent-resource-source-host">{new URL(source.url).hostname}</span>{source.excerpt ? <span className="agent-resource-source-snippet">{source.excerpt}</span> : null}</span>
        <ChevronRight className="agent-resource-source-chevron" size={15} aria-hidden="true" />
      </AgentReplySourcePreview>
    </li>)}</ul>
    <p className="agent-resource-note agent-resource-boundary"><Info size={13} aria-hidden="true" /><span>{t('agentResources.sourceBoundary')}</span></p>
  </section>;
}

function ResourceFields({ value }: { value: Json }) {
  const { t } = useLocale();
  if (Array.isArray(value)) return value.length ? <ol className="agent-resource-array">{value.map((item, index) => <li key={index}><ResourceFields value={item} /></li>)}</ol> : <code>[]</code>;
  if (value !== null && typeof value === 'object') return Object.keys(value).length ? <dl className="agent-resource-fields">{Object.entries(value).map(([key, item]) => <div key={key}><dt>{key}</dt><dd><ResourceFields value={item} /></dd></div>)}</dl> : <code>{'{}'}</code>;
  return <pre className="agent-resource-text">{value === '[binary content omitted]' ? t('agentResources.binaryOmitted') : value === null ? 'null' : String(value)}</pre>;
}

function StructuredResource({ value }: { value: Json }) {
  const { t } = useLocale();
  const [expanded, setExpanded] = useState(false);
  const fields = useMemo(() => expanded ? boundedResourceFields(value) : undefined, [expanded, value]);
  return <details className="agent-resource-structured" onToggle={event => setExpanded(event.currentTarget.open)}>
    <summary><FileJson size={14} aria-hidden="true" /><span>{t('agentResources.fields')}</span><ChevronDown className="agent-resource-disclosure-chevron" size={14} aria-hidden="true" /></summary>
    {fields ? <><ResourceFields value={fields.value} />{fields.truncated ? <p className="agent-resource-warning">{t('agentResources.fieldsTruncated', { count: NATIVE_RESOURCE_LIMITS.fields, depth: NATIVE_RESOURCE_LIMITS.depth })}</p> : null}</> : null}
  </details>;
}

function ReceivedText({ text }: { text: string }) {
  const { t } = useLocale();
  const parsed = useMemo(() => {
    if (!/^[\s]*[\[{]/.test(text)) return undefined;
    try { return JSON.parse(text) as Json; } catch { return undefined; }
  }, [text]);
  if (/^data:[^,]{0,128};base64,/.test(text) || text.length > 256 && /^[A-Za-z0-9+/=\r\n]+$/.test(text.slice(0, 512))) return <p className="agent-resource-note">{t('agentResources.binaryOmitted')}</p>;
  return parsed === undefined ? <pre className="agent-resource-text">{text}</pre> : <StructuredResource value={parsed} />;
}

export interface AgentResourcePreview { base64: string; mime: string }
export type AgentNativeResourceActions = Pick<AgentReferenceActions, 'onOpenReference' | 'onPreviewResource'>;
type ImagePreviewState = { input?: string; mime?: string; status: 'idle' | 'loading' | 'ready' | 'failed'; url?: string; width?: number; height?: number; error?: NativeImageFailure | 'decodeFailed' | 'previewExpired' };
function ReceivedImage({ base64, previewId, mime, title, onPreviewResource }: { base64?: string; previewId?: string; mime: string; title: string } & Pick<AgentNativeResourceActions, 'onPreviewResource'>) {
  const { t } = useLocale();
  const input = previewId || base64 || '';
  const loader = useRef(onPreviewResource);
  loader.current = onPreviewResource;
  const [request, setRequest] = useState<{ input: string; mime: string; base64?: string; previewId?: string }>();
  const requested = request?.input === input && request?.mime === mime && request?.base64 === base64 && request?.previewId === previewId;
  useEffect(() => { setRequest(undefined); }, [input, base64, previewId, mime]);
  const [preview, setPreview] = useState<ImagePreviewState>({ status: 'idle' });
  const current = preview.input === input && preview.mime === mime ? preview : { status: 'idle' as const };
  useEffect(() => {
    if (!requested) return;
    let active = true;
    let objectUrl: string | undefined;
    const image = new Image();
    setPreview({ input, mime, status: 'loading' });
    async function load() {
      let payload: AgentResourcePreview;
      if (previewId) {
        try {
          if (!loader.current) throw new Error('previewUnavailable');
          payload = await loader.current(previewId);
        } catch {
          if (active) setPreview({ input, mime, status: 'failed', error: 'previewExpired' });
          return;
        }
      } else payload = { base64: base64 ?? '', mime };
      if (!active) return;
      try {
        if (mime && payload.mime !== mime) throw new NativeImageError('mimeMismatch');
        const decoded = decodeNativeImage(payload.base64, payload.mime);
        objectUrl = URL.createObjectURL(new Blob([decoded.bytes], { type: payload.mime }));
        const url = objectUrl;
        image.onload = () => {
          if (!active) return;
          if (!(image.naturalWidth === decoded.width && image.naturalHeight === decoded.height || image.naturalWidth === decoded.height && image.naturalHeight === decoded.width)) {
            URL.revokeObjectURL(url); objectUrl = undefined;
            setPreview({ input, mime, status: 'failed', error: 'invalidDimensions' });
            return;
          }
          setPreview({ input, mime, status: 'ready', url, width: image.naturalWidth, height: image.naturalHeight });
        };
        image.onerror = () => {
          if (!active) return;
          URL.revokeObjectURL(url); objectUrl = undefined;
          setPreview({ input, mime, status: 'failed', error: 'decodeFailed' });
        };
        image.src = url;
      } catch (error) {
        if (active) setPreview({ input, mime, status: 'failed', error: error instanceof NativeImageError ? error.reason : 'decodeFailed' });
      }
    }
    void load();
    return () => { active = false; image.onload = null; image.onerror = null; image.removeAttribute('src'); if (objectUrl) URL.revokeObjectURL(objectUrl); };
  }, [requested, input, base64, previewId, mime]);
  return <div className="agent-resource-image-preview">
    <div className="agent-resource-preview-toolbar">
      {!requested ? <Button type="button" size="sm" variant="secondary" onClick={() => setRequest({ input, mime, base64, previewId })}><Eye size={14} aria-hidden="true" />{t('agentResources.previewImage')}</Button> : <Button type="button" size="sm" variant="secondary" onClick={() => { setRequest(undefined); setPreview({ status: 'idle' }); }}><EyeOff size={14} aria-hidden="true" />{t('agentResources.hidePreview')}</Button>}
      <details className="agent-resource-technical"><summary><Info size={13} aria-hidden="true" />{t('agent.details')}</summary><p className="agent-resource-note">{t('agentResources.imageLimits')}</p></details>
    </div>
    {requested && current.status === 'loading' ? <p role="status" className="agent-resource-loading">{t('agentResources.loadingImage')}</p> : null}
    {requested && current.status === 'failed' ? <p role="alert" className="agent-resource-warning">{t(`agentResources.${current.error}`)}</p> : null}
    {requested && current.status === 'ready' && current.url ? <figure><img src={current.url} alt={title || t('agentResources.imageAlt')} width={current.width} height={current.height} /><figcaption>{current.width} × {current.height}</figcaption></figure> : null}
  </div>;
}

function ResourceLocation({ resource, onOpenReference }: { resource: NativeResource; onOpenReference?: AgentReferenceActions['onOpenReference'] }) {
  const { t } = useLocale();
  const location = resource.path || resource.uri || '';
  const path = location && (resource.path || !/^[a-z][a-z\d+.-]*:/i.test(location) || location.startsWith('file:')) ? nativeResourcePath(location) : null;
  const url = !path && location ? safeSourceUrl(location) : null;
  const generation = useRef(0);
  const openingRequest = useRef<number | undefined>(undefined);
  const [opening, setOpening] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => { generation.current++; openingRequest.current = undefined; setOpening(false); setFailed(false); return () => { generation.current++; }; }, [location]);
  if (!location) return null;
  return <div className="agent-resource-location">
    <div className="agent-resource-location-bar">
    <FolderOpen size={15} aria-hidden="true" /><code>{location}</code>
    {url ? <ConfirmedExternalLink url={url}><ExternalLink size={14} aria-hidden="true" />{t('agentResources.openExternal')}</ConfirmedExternalLink> : path ? <Button type="button" size="sm" variant="secondary" disabled={opening || !onOpenReference} onClick={async () => {
        if (!onOpenReference || openingRequest.current !== undefined) return;
        const request = generation.current;
        const current = () => generation.current === request;
        openingRequest.current = request;
        setOpening(true); setFailed(false);
        try { await onOpenReference({ path, source: 'tool', diff: false }, 'file', current); }
        catch { if (current()) setFailed(true); }
        finally { if (current()) { openingRequest.current = undefined; setOpening(false); } }
      }}>{t(opening ? 'agentResources.openingFile' : 'agentResources.openFile')}</Button> : null}
    </div>
    <p className="agent-resource-note">{t(url ? 'agentResources.remoteBoundary' : path ? onOpenReference ? 'agentResources.fileBoundary' : 'agentResources.fileUnavailable' : 'agentResources.unsupportedLocation')}</p>
    {failed ? <p role="alert" className="agent-resource-warning">{t('agentResources.fileOpenFailed')}</p> : null}
  </div>;
}

function ResourceCard({ resource, onOpenReference, onPreviewResource }: { resource: NativeResource } & AgentNativeResourceActions) {
  const { t } = useLocale();
  const image = resource.kind === 'image' || resource.mime.startsWith('image/');
  const previewable = !!(resource.base64 || resource.previewId && onPreviewResource) && ['image/png', 'image/jpeg', 'image/webp'].includes(resource.mime);
  const Icon = image ? FileImage : resource.kind === 'structured' ? FileJson : resource.kind === 'resource_link' ? Link : FileText;
  const omittedKey = resource.omitted === 'image_payload_omitted' ? 'agentResources.imagePayloadOmitted' : resource.omitted === 'document_content_shown_below' ? 'agentResources.documentContent' : 'agentResources.payloadOmitted';
  return <article className="agent-resource-card" data-resource-kind={resource.kind}>
    <header><span className="agent-resource-heading-icon"><Icon size={18} aria-hidden="true" /></span><strong>{resource.title || t(`agentResources.kind.${resource.kind}`)}</strong>{resource.title ? <span className="agent-resource-kind">{t(`agentResources.kind.${resource.kind}`)}</span> : null}</header>
    <dl className="agent-resource-metadata">
      <div><dt>{t('agentResources.type')}</dt><dd>{resource.nativeType || resource.kind}{resource.mime ? ` · ${resource.mime}` : ''}</dd></div>
      {resource.kind !== 'text' && resource.kind !== 'structured' ? <div><dt>{t('agentResources.size')}</dt><dd>{resource.byteLength === undefined ? t('agentResources.sizeUnknown') : t('agentResources.bytes', { count: resource.byteLength })}</dd></div> : null}
    </dl>
    {resource.omitted ? <div className="agent-resource-omitted"><p className="agent-resource-note">{t(omittedKey)}</p><details className="agent-resource-technical"><summary>{t('agent.details')}</summary><code className="agent-resource-native-reason">{resource.omitted}</code></details></div> : null}
    {resource.text !== undefined ? resource.text ? <ReceivedText text={resource.text} /> : <p className="agent-resource-note">{t('agentResources.emptyText')}</p> : null}
    {resource.fields !== undefined ? <StructuredResource value={resource.fields} /> : null}
    {previewable ? <ReceivedImage base64={resource.base64} previewId={resource.previewId} mime={resource.mime} title={resource.title} onPreviewResource={onPreviewResource} /> : null}
    {resource.previewId && !onPreviewResource ? <p className="agent-resource-note">{t('agentResources.previewUnavailable')}</p> : null}
    {!['image/png', 'image/jpeg', 'image/webp'].includes(resource.mime) && (resource.base64 || resource.previewId || image || resource.mime === 'application/pdf') ? <p className="agent-resource-note">{t(image ? 'agentResources.unsupportedMime' : 'agentResources.binaryBoundary')}</p> : null}
    {!resource.text && resource.fields === undefined && !resource.base64 && !resource.previewId && !resource.omitted && !resource.path && !resource.uri ? <p className="agent-resource-note">{t('agentResources.noPayload')}</p> : null}
    {resource.truncated ? <p className="agent-resource-warning">{t('agentResources.textTruncated', { count: resource.originalCharacters ?? NATIVE_RESOURCE_LIMITS.text, limit: NATIVE_RESOURCE_LIMITS.text })}</p> : null}
    <ResourceLocation resource={resource} onOpenReference={onOpenReference} />
    {resource.kind === 'unsupported' ? <p className="agent-resource-note">{t('agentResources.unsupportedType')}</p> : null}
  </article>;
}

export function AgentNativeResources({ value, onOpenReference, onPreviewResource }: { value: Json } & AgentNativeResourceActions) {
  const { t } = useLocale();
  const result = useMemo(() => nativeResources(value), [value]);
  return <section className="agent-native-resources" aria-label={t('agentResources.resources')}>
    {result.resources.map((resource, index) => <ResourceCard key={`${index}:${resource.kind}:${resource.path ?? resource.uri ?? resource.previewId ?? ''}`} resource={resource} onOpenReference={onOpenReference} onPreviewResource={onPreviewResource} />)}
    {!result.resources.length ? <p className="agent-resource-note">{t('agentResources.noPayload')}</p> : null}
    {result.truncated ? <p className="agent-resource-warning">{t('agentResources.itemsTruncated', { count: NATIVE_RESOURCE_LIMITS.items })}</p> : null}
    <p className="agent-resource-note agent-resource-boundary"><Info size={13} aria-hidden="true" /><span>{t('agentResources.resourceBoundary')}</span></p>
  </section>;
}

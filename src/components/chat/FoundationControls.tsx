import { useEffect, useRef, useState } from 'react';
import { Button, Checkbox, Icon, Select, Stack } from '@dgos/ui';
import { t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore } from '../../stores/chatStore';
import { fileBase64 } from './Attachments';
import { ChatNotice } from './ChatNotice';
import { readResourcePreview, selectableResourceText, selectedResourceRange, type ResourcePreview, type SelectableResourceText } from './resourcePreview';
import type { AiContextRef, AiResource } from '../../types/ai';
import './chatResources.css';

function useResourceClock() {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 30_000); return () => window.clearInterval(timer); }, []);
  return now;
}

function expiryLabel(resource: AiResource, now: number) {
  const minutes = Math.ceil((resource.expires_at * 1000 - now) / 60_000);
  return minutes <= 0 ? t('chat.materialExpired') : minutes < 60 ? t('chat.materialExpiresMinutes', { count: minutes }) : minutes < 1440 ? t('chat.materialExpiresHours', { count: Math.ceil(minutes / 60) }) : t('chat.materialExpiresDays', { count: Math.ceil(minutes / 1440) });
}

function referenceCoverage(resource: AiResource, ref: AiContextRef) {
  return ref.sha256 === resource.sha256 && ref.start === 0 && ref.max_bytes >= resource.bytes_len ? t('chat.materialWhole') : t('chat.materialExcerpt');
}

function updateReferences(sessionId: string, update: (refs: AiContextRef[]) => AiContextRef[]) {
  const state = useChatStore.getState();
  const settings = state.sessionFoundation[sessionId];
  state.setFoundation(sessionId, { output_schema: settings?.output_schema ?? null, context_refs: update(settings?.context_refs ?? []) });
}

export function ResourceReferenceChips({ sessionId, disabled, onEdit }: { sessionId: string; disabled: boolean; onEdit: () => void }) {
  useLocale();
  const resources = useChatStore(state => state.sessionResources[sessionId]);
  const settings = useChatStore(state => state.sessionFoundation[sessionId]);
  const now = useResourceClock();
  if (!settings?.context_refs.length) return null;
  return <section className="chat-material-chips" aria-label={t('chat.materialSelected')}>
    {settings.context_refs.map(ref => {
      const resource = resources?.find(item => item.id === ref.resource_id);
      const unavailable = !resource || resource.expires_at * 1000 <= now || resource.sha256 !== ref.sha256;
      const name = resource?.name ?? t('chat.materialUnavailable');
      return <div className="chat-context-chip chat-material-chip" key={ref.resource_id} data-unavailable={unavailable || undefined}>
        <Icon name="file" size="sm" />
        <Button size="xs" variant="ghost" disabled={disabled} className="chat-material-chip-label" onClick={onEdit} aria-label={t('chat.materialEditReference', { name })}>
          <span>{name}</span><small>{unavailable ? t('chat.materialUnavailable') : `${referenceCoverage(resource, ref)} · ${ref.mode === 'on_demand' ? t('chat.materialOnDemandShort') : t('chat.materialInlineShort')}`}</small>
        </Button>
        <Button size="xxs" variant="ghost" iconOnly disabled={disabled} aria-label={t('chat.materialRemoveReference', { name })} title={t('chat.materialRemoveReference', { name })} onClick={() => updateReferences(sessionId, refs => refs.filter(item => item.resource_id !== ref.resource_id))}><Icon name="close" size="sm" /></Button>
      </div>;
    })}
  </section>;
}

function ResourceItem({ sessionId, resource, reference, disabled, busy, now, onDelete }: {
  sessionId: string; resource: AiResource; reference?: AiContextRef; disabled: boolean; busy: boolean; now: number; onDelete: () => void;
}) {
  const [preview, setPreview] = useState<ResourcePreview | null>(null);
  const [text, setText] = useState<SelectableResourceText | null>(null);
  const [reading, setReading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [range, setRange] = useState<{ start: number; max_bytes: number } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const request = useRef(0);
  const expired = resource.expires_at * 1000 <= now;
  const image = resource.mime.startsWith('image/');
  useEffect(() => () => { request.current++; }, []);
  useEffect(() => {
    const unsubscribe = useChatStore.subscribe(state => {
      if (state.activeSessionId === sessionId && state.sessionResources[sessionId]?.includes(resource)) return;
      request.current++; setPreview(null); setText(null); setRange(null); setReading(false);
    });
    return unsubscribe;
  }, [sessionId, resource]);
  const select = (selection: { start: number; max_bytes: number }, mode = reference?.mode ?? 'inline') => {
    if (disabled || busy || expired || selection.max_bytes <= 0) return;
    updateReferences(sessionId, refs => {
      const next: AiContextRef = { resource_id: resource.id, sha256: resource.sha256, ...selection, mode: image ? 'inline' : mode };
      return refs.some(ref => ref.resource_id === resource.id) ? refs.map(ref => ref.resource_id === resource.id ? next : ref) : [...refs, next];
    });
  };
  async function read() {
    if (reading || disabled || busy || expired) return;
    const token = ++request.current;
    setReading(true); setError(null);
    try {
      const result = await readResourcePreview(sessionId, resource);
      if (token !== request.current || !useChatStore.getState().sessionResources[sessionId]?.includes(resource)) return;
      const bytes = Uint8Array.from(atob(result.base64), char => char.charCodeAt(0));
      if (bytes.length !== result.bytes) throw new Error('resource_preview_changed');
      if (!image) setText(selectableResourceText(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes)));
      setPreview(result);
    } catch (failure) { if (token === request.current) setError(chatErrorText(failure)); }
    finally { if (token === request.current) setReading(false); }
  }
  const blocked = disabled || busy;
  return <article className="chat-material-resource">
    <div className="chat-material-resource-heading"><strong>{resource.name}</strong><span className="chat-material-expiry" data-expired={expired || undefined} title={new Date(resource.expires_at * 1000).toLocaleString()}>{expiryLabel(resource, now)}</span></div>
    <Checkbox label={t('chat.materialSelectWhole', { name: resource.name })} checked={!!reference && reference.start === 0 && reference.max_bytes >= resource.bytes_len && reference.sha256 === resource.sha256} disabled={blocked || expired || !resource.bytes_len} onChange={checked => checked ? select({ start: 0, max_bytes: resource.bytes_len }) : updateReferences(sessionId, refs => refs.filter(ref => ref.resource_id !== resource.id))} />
    {reference ? <div className="chat-material-reference">
      <span>{referenceCoverage(resource, reference)}</span>
      <Button size="xs" variant="ghost" disabled={blocked} onClick={() => updateReferences(sessionId, refs => refs.filter(ref => ref.resource_id !== resource.id))}>{t('chat.materialRemoveReferenceShort')}</Button>
    </div> : <p className="chat-material-help">{t('chat.materialNotSelected')}</p>}
    {reference && !image ? <><Select label={t('chat.materialDelivery')} value={reference.mode} disabled={blocked || expired} options={[{ value: 'inline', label: t('chat.materialInlineShort') }, { value: 'on_demand', label: t('chat.materialOnDemandShort') }]} onChange={mode => select({ start: reference.start, max_bytes: reference.max_bytes }, mode as AiContextRef['mode'])} /><p className="chat-material-help">{reference.mode === 'on_demand' ? t('chat.materialOnDemandHelp') : t('chat.materialInlineHelp')}</p></> : image ? <p className="chat-material-help">{t('chat.materialImageHelp')}</p> : null}
    <div className="chat-material-actions"><Button size="xs" variant="secondary" disabled={blocked || expired || reading} onClick={() => { if (preview) { request.current++; setPreview(null); setText(null); setRange(null); } else void read(); }}>{reading ? t('assets.loading') : preview ? t('chat.materialHidePreview') : image ? t('chat.materialPreviewImage') : t('chat.materialPreview')}</Button></div>
    {preview ? <div className="chat-material-preview">
      {preview.truncated ? <ChatNotice tone="warning">{t('chat.materialTruncated')}</ChatNotice> : null}
      {image ? <img src={`data:${resource.mime};base64,${preview.base64}`} alt={resource.name} /> : text ? <>
        <label className="chat-material-preview-label">{t('chat.materialSelectText')}<textarea readOnly value={text.text} rows={9} spellCheck={false} disabled={blocked || expired} onSelect={event => setRange(selectedResourceRange(text, event.currentTarget.selectionStart, event.currentTarget.selectionEnd))} /></label>
        <Button size="xs" variant="secondary" disabled={blocked || expired || !range} onClick={() => { if (range) select({ start: preview.start + range.start, max_bytes: range.max_bytes }); }}>{t('chat.materialUseSelection')}</Button>
        <p className="chat-material-help">{range ? t('chat.materialSelectionReady') : t('chat.materialSelectionHelp')}</p>
      </> : null}
    </div> : null}
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
    <details className="chat-material-diagnostics"><summary>{t('chat.materialDiagnostics')}</summary><dl><dt>{t('chat.materialHash')}</dt><dd>{resource.sha256}</dd><dt>{t('chat.materialSize')}</dt><dd>{resource.bytes_len} {t('chat.byteUnit')}</dd><dt>{t('chat.materialExpiresAt')}</dt><dd>{new Date(resource.expires_at * 1000).toLocaleString()}</dd>{reference ? <><dt>{t('chat.materialByteRange')}</dt><dd>{reference.start}–{Math.min(resource.bytes_len, reference.start + reference.max_bytes)} ({reference.max_bytes} {t('chat.byteUnit')})</dd></> : null}</dl></details>
    {confirmDelete ? <div className="chat-material-delete-confirm"><p>{t('chat.materialDeleteHelp')}</p><div className="chat-material-actions"><Button size="xs" variant="danger" disabled={blocked} onClick={onDelete}>{t('chat.materialConfirmDelete')}</Button><Button size="xs" variant="ghost" disabled={blocked} onClick={() => setConfirmDelete(false)}>{t('common.cancel')}</Button></div></div> : <Button size="xs" variant="ghost" disabled={blocked} onClick={() => setConfirmDelete(true)}>{t('chat.materialDeleteResource')}</Button>}
  </article>;
}

/** Importing never selects a resource. References stay fixed to its hash and exact byte subset. */
export default function FoundationControls({ sessionId, disabled, onBusy }: { sessionId: string; disabled: boolean; onBusy?: (busy: boolean) => void }) {
  useLocale();
  const resources = useChatStore(state => state.sessionResources[sessionId]);
  const settings = useChatStore(state => state.sessionFoundation[sessionId]);
  const importResource = useChatStore(state => state.importResource);
  const deleteResource = useChatStore(state => state.deleteResource);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const pending = useRef(false);
  const picker = useRef<HTMLInputElement>(null);
  const now = useResourceClock();
  useEffect(() => { alive.current = true; return () => { alive.current = false; onBusy?.(false); }; }, []);
  const perform = async (action: () => Promise<unknown>) => {
    if (pending.current || disabled) return;
    pending.current = true; setBusy(true); setError(null); onBusy?.(true);
    try { await action(); } catch (failure) { if (alive.current) setError(failure instanceof Error && failure.message === 'resource_limits' ? t('chat.resourceImportLimits') : chatErrorText(failure)); }
    finally { pending.current = false; if (alive.current) { setBusy(false); onBusy?.(false); } }
  };
  return <Stack gap={3}>
    <div><h4 className="m-0 text-sm font-semibold">{t('chat.materialManagedTitle')}</h4><p className="chat-material-help">{t('chat.materialManagedHelp')}</p></div>
    <input ref={picker} aria-label={t('chat.materialImportResource')} type="file" hidden accept=".txt,.md,text/plain,text/markdown,image/png,image/jpeg,image/webp" disabled={disabled || busy} onChange={event => {
      const file = event.target.files?.[0]; event.target.value = '';
      if (!file) return;
      void perform(async () => {
        const mime = /\.md$/i.test(file.name) ? 'text/markdown' : /\.txt$/i.test(file.name) ? 'text/plain' : file.type;
        const limit = mime === 'text/plain' || mime === 'text/markdown' ? 1024 * 1024 : ['image/png', 'image/jpeg', 'image/webp'].includes(mime) ? 8 * 1024 * 1024 : 0;
        if (!limit || file.size > limit || file.size === 0) throw new Error('resource_limits');
        const base64 = await fileBase64(file);
        if (!alive.current) return;
        await importResource(sessionId, { name: file.name, mime, base64 });
      });
    }} />
    <Button type="button" size="sm" variant="secondary" disabled={disabled || busy} onClick={() => picker.current?.click()}>{t('chat.materialImportResource')}</Button>
    {(resources ?? []).map(resource => <ResourceItem key={resource.id} sessionId={sessionId} resource={resource} reference={settings?.context_refs.find(ref => ref.resource_id === resource.id)} disabled={disabled} busy={busy} now={now} onDelete={() => void perform(() => deleteResource(sessionId, resource.id))} />)}
    {!resources?.length ? <p className="chat-material-help">{t('chat.materialEmpty')}</p> : null}
    {busy ? <p role="status" className="m-0 text-xs">{t('chat.resourceOperation')}</p> : null}
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
  </Stack>;
}

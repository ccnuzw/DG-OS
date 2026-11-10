import { useEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input, Select, Stack } from '@dgos/ui';
import { t, useLocale } from '../../i18n';
import { chatErrorText, useChatStore } from '../../stores/chatStore';
import { fileBase64 } from './Attachments';
import { ChatNotice } from './ChatNotice';
import type { AiContextRef } from '../../types/ai';

/** Explicit per-session selection only; importing a source never includes it automatically. */
export default function FoundationControls({ sessionId, disabled }: { sessionId: string; disabled: boolean }) {
  useLocale();
  const resources = useChatStore(state => state.sessionResources[sessionId]);
  const settings = useChatStore(state => state.sessionFoundation[sessionId]);
  const setFoundation = useChatStore(state => state.setFoundation);
  const importResource = useChatStore(state => state.importResource);
  const deleteResource = useChatStore(state => state.deleteResource);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const pending = useRef(false);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const refs = settings?.context_refs ?? [];
  const schema = settings?.output_schema ?? null;
  let invalidSchema = false;
  if (schema !== null) { try { JSON.parse(schema); } catch { invalidSchema = true; } }
  const patch = (next: Partial<NonNullable<typeof settings>>) => setFoundation(sessionId, { context_refs: refs, output_schema: schema, ...next });
  const edit = (id: string, next: Partial<AiContextRef>) => patch({ context_refs: refs.map(ref => ref.resource_id === id ? { ...ref, ...next } : ref) });
  const perform = async (action: () => Promise<unknown>) => {
    if (pending.current || disabled) return;
    pending.current = true; setBusy(true); setError(null);
    try { await action(); } catch (failure) { if (alive.current) setError(failure instanceof Error && failure.message === 'resource_limits' ? t('chat.resourceImportLimits') : chatErrorText(failure)); }
    finally { pending.current = false; if (alive.current) setBusy(false); }
  };
  return <Stack gap={3}>
    <h4 className="m-0 text-sm font-semibold">{t('chat.foundationResources')}</h4>
    <p className="m-0 text-xs text-ink-secondary">{t('chat.foundationResourceHelp')}</p>
    <label className="text-xs">{t('chat.importFoundationResource')}<input type="file" accept=".txt,.md,text/plain,text/markdown,image/png,image/jpeg,image/webp" disabled={disabled || busy} onChange={event => {
      const file = event.target.files?.[0]; event.target.value = '';
      if (!file) return;
      void perform(async () => {
        const mime = /\.md$/i.test(file.name) ? 'text/markdown' : /\.txt$/i.test(file.name) ? 'text/plain' : file.type;
        const limit = mime === 'text/plain' || mime === 'text/markdown' ? 1024 * 1024 : ['image/png', 'image/jpeg', 'image/webp'].includes(mime) ? 8 * 1024 * 1024 : 0;
        if (!limit || file.size > limit) throw new Error('resource_limits');
        const base64 = await fileBase64(file);
        if (!alive.current) return;
        await importResource(sessionId, { name: file.name, mime, base64 });
      });
    }} /></label>
    {(resources ?? []).map(resource => {
      const ref = refs.find(item => item.resource_id === resource.id);
      const image = resource.mime.startsWith('image/');
      return <fieldset key={resource.id} className="m-0 min-w-0 border-0 p-0" disabled={disabled || busy}>
        <legend className="text-xs wrap-anywhere">{resource.name}</legend>
        <Stack gap={2}>
          <Checkbox label={t('chat.selectFoundationResource', { name: resource.name })} checked={!!ref} onChange={checked => patch({ context_refs: checked ? [...refs, { resource_id: resource.id, sha256: resource.sha256, start: 0, max_bytes: image ? resource.bytes_len : Math.min(resource.bytes_len, 8192), mode: 'inline' }] : refs.filter(item => item.resource_id !== resource.id) })} />
          <p className="m-0 text-xs wrap-anywhere">{resource.sha256} · {resource.bytes_len} {t('chat.byteUnit')} · {t('chat.resourceExpiry', { time: new Date(resource.expires_at * 1000).toLocaleString() })}</p>
          {ref ? <>
            <Input label={t('chat.resourceStart', { name: resource.name })} type="number" min={0} step={1} value={ref.start} disabled={disabled || busy || image} onChange={event => edit(resource.id, { start: Number(event.target.value) })} />
            <Input label={t('chat.resourceBytes', { name: resource.name })} type="number" min={1} step={1} value={ref.max_bytes} disabled={disabled || busy || image} onChange={event => edit(resource.id, { max_bytes: Number(event.target.value) })} />
            <Select label={t('chat.resourceMode', { name: resource.name })} disabled={disabled || busy} value={ref.mode} options={[{ value: 'inline', label: t('chat.resourceInline') }, { value: 'on_demand', label: t('chat.resourceOnDemand') }]} onChange={mode => edit(resource.id, { mode: mode as AiContextRef['mode'] })} />
          </> : null}
          <Button type="button" size="sm" variant="secondary" disabled={disabled || busy} onClick={() => void perform(() => deleteResource(sessionId, resource.id))}>{t('chat.deleteFoundationResource')}</Button>
        </Stack>
      </fieldset>;
    })}
    <Checkbox label={t('chat.structuredOutput')} checked={schema !== null} disabled={disabled} onChange={checked => patch({ output_schema: checked ? '' : null })} />
    {schema !== null ? <Input label={t('chat.outputSchema')} multiline rows={5} value={schema} disabled={disabled} onChange={event => patch({ output_schema: event.target.value })} /> : null}
    <p className="m-0 text-xs text-ink-secondary">{t('chat.outputSchemaHelp')}</p>
    {invalidSchema ? <ChatNotice tone="error">{t('chat.invalidOutputSchema')}</ChatNotice> : null}
    {busy ? <p role="status" className="m-0 text-xs">{t('chat.resourceOperation')}</p> : null}
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
  </Stack>;
}

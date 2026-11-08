import { useEffect, useRef, useState, type ReactNode } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Icon, Modal, Popover } from '@dgos/ui';
import { Paperclip } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useChatStore, type ChatAsset } from '../../stores/chatStore';

export async function assetCall<T>(command: string, args: Record<string, unknown>): Promise<T> {
  const result = await invoke<{ success: boolean; data: T }>(command, args);
  if (!result.success) throw new Error('asset_operation_failed');
  return result.data;
}
export function fileBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('file_read_failed'));
    reader.onload = () => resolve(String(reader.result).split(',')[1] ?? '');
    reader.readAsDataURL(blob);
  });
}

export function AssetPreview({ asset, disabled = false }: { asset: ChatAsset; disabled?: boolean }) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState(false);
  const request = useRef(0);
  useEffect(() => () => { request.current++; }, []);
  async function preview() {
    const token = ++request.current;
    setOpen(true); setContent(null); setError(false);
    try {
      const result = await assetCall<{ asset: ChatAsset; base64: string }>('preview_chat_asset', { assetId: asset.id });
      if (token !== request.current) return;
      if (asset.mime.startsWith('image/')) setContent(`data:${asset.mime};base64,${result.base64}`);
      else setContent(new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(atob(result.base64), char => char.charCodeAt(0))));
    } catch { if (token === request.current) setError(true); }
  }
  return <><Button size="xs" variant="ghost" className="chat-asset-preview" disabled={disabled} onClick={() => void preview()}>{t('assets.previewLabel', { name: asset.name, kib: Math.ceil(asset.bytes_len / 1024) })}</Button>
    <Modal kind="alert" open={open} onOpenChange={value => { setOpen(value); if (!value) { request.current++; setContent(null); } }} title={asset.name}>
      {error ? <p role="alert">{t('assets.error')}</p> : content === null ? <p role="status">{t('assets.loading')}</p> : asset.mime.startsWith('image/') ? <img src={content} alt={asset.name} className="max-h-96 max-w-full object-contain" /> : <pre className="max-h-96 overflow-auto whitespace-pre-wrap break-words text-sm">{content}</pre>}
    </Modal></>;
}

export default function Attachments({ sessionId, disabled, onBusy, children }: {
  sessionId: string;
  disabled: boolean;
  onBusy: (busy: boolean) => void;
  children: (slots: { contextTrigger: ReactNode; attachmentTrigger: ReactNode; attachments: ReactNode }) => ReactNode;
}) {
  const { t } = useLocale();
  const assets = useChatStore(state => state.sessionAssets[sessionId]);
  const [operation, setOperation] = useState<'import' | 'remove' | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const picker = useRef<HTMLInputElement>(null);
  const version = useRef(0);
  const activeOperation = useRef<'import' | 'remove' | null>(null);
  useEffect(() => {
    const unsubscribe = useChatStore.subscribe(state => {
      if (state.activeSessionId === sessionId && state.sessions.some(session => session.id === sessionId)) return;
      version.current++; updateOperation(null); setOpen(false);
    });
    return () => { unsubscribe(); version.current++; activeOperation.current = null; onBusy(false); };
  }, [sessionId]);
  const busy = operation !== null;
  const updateOperation = (value: 'import' | 'remove' | null) => { activeOperation.current = value; setOperation(value); onBusy(value !== null); };
  async function add(files: File[]) {
    if (activeOperation.current || disabled) return;
    const current = useChatStore.getState().sessionAssets[sessionId] ?? [];
    if (current.length + files.length > 4 || current.reduce((sum, asset) => sum + asset.bytes_len, 0) + files.reduce((sum, file) => sum + file.size, 0) > 24 * 1024 * 1024) { setError(t('assets.limits')); return; }
    const token = ++version.current;
    updateOperation('import'); setError(null);
    try {
      for (const file of files) {
        const mime = file.type.startsWith('text/') || /\.(txt|md|csv|json|log|ya?ml)$/i.test(file.name) ? 'text/plain' : file.type;
        if (!['text/plain', 'image/png', 'image/jpeg', 'image/webp'].includes(mime) || file.size > (mime === 'text/plain' ? 1 : 8) * 1024 * 1024) throw new Error('limits');
        const base64 = await fileBase64(file);
        if (token !== version.current) break;
        const asset = await assetCall<ChatAsset>('import_chat_asset', { input: { name: file.name, mime, base64 } });
        if (token !== version.current || !useChatStore.getState().sessions.some(session => session.id === sessionId)) { void assetCall('remove_chat_asset', { assetId: asset.id }).catch(() => undefined); break; }
        const state = useChatStore.getState();
        state.setSessionAssets(sessionId, [...(state.sessionAssets[sessionId] ?? []), asset]);
      }
    } catch { if (token === version.current) setError(t('assets.error')); }
    finally { if (token === version.current) updateOperation(null); }
  }
  async function remove(asset: ChatAsset) {
    if (activeOperation.current || disabled) return;
    const token = ++version.current; updateOperation('remove'); setError(null);
    try {
      await assetCall('remove_chat_asset', { assetId: asset.id });
      // A completed deletion still belongs to its original draft after a tab switch.
      const state = useChatStore.getState();
      if (state.sessionAssets[sessionId]?.includes(asset)) state.setSessionAssets(sessionId, state.sessionAssets[sessionId].filter(item => item.id !== asset.id));
    } catch { if (token === version.current) setError(t('assets.error')); }
    finally { if (token === version.current) updateOperation(null); }
  }
  const cannotAdd = disabled || busy || (assets?.length ?? 0) >= 4;
  const cancelImport = () => { version.current++; updateOperation(null); };
  const attachmentTrigger = <Button size="sm" variant="ghost" iconOnly className="chat-composer-icon" disabled={cannotAdd} aria-label={t('assets.add')} title={t('assets.add')} onClick={() => picker.current?.click()}><Paperclip size={16} aria-hidden="true" /></Button>;
  const contextTrigger = <Popover label={t('assets.context')} side="top" align="start" open={open} onOpenChange={setOpen} trigger={
    <Button size="sm" variant="ghost" iconOnly className="chat-composer-icon" disabled={disabled} aria-label={t('assets.context')} title={t('assets.context')}><Icon name="plus" /></Button>
  } content={
    <section className="chat-context-menu">
      <header className="chat-picker-header"><strong>{t('assets.context')}</strong><Button size="xxs" variant="ghost" iconOnly aria-label={t('common.close')} title={t('common.close')} onClick={() => setOpen(false)}><Icon name="close" size="sm" /></Button></header>
      <Button size="sm" variant="ghost" className="chat-context-add" disabled={cannotAdd} onClick={() => { setOpen(false); picker.current?.click(); }}><Paperclip size={16} aria-hidden="true" />{t('assets.add')}</Button>
      <p className="chat-picker-help">{t('assets.limits')}</p>
      {operation === 'import' && <Button size="xs" variant="secondary" onClick={cancelImport}>{t('assets.cancelImport')}</Button>}
    </section>
  } />;
  const attachments = assets?.length || operation === 'import' || error ? <section aria-label={t('assets.title')} className="chat-attachment-context" aria-busy={busy}>
    {assets?.map(asset => <div key={asset.id} className="chat-context-chip"><Icon name="file" size="sm" /><AssetPreview asset={asset} disabled={busy} /><Button size="xxs" variant="ghost" iconOnly className="chat-context-remove" disabled={disabled || busy} aria-label={t('assets.remove', { name: asset.name })} title={t('assets.remove', { name: asset.name })} onClick={() => void remove(asset)}><Icon name="close" size="sm" /></Button></div>)}
    {operation === 'import' && <Button size="xs" variant="secondary" onClick={cancelImport}>{t('assets.cancelImport')}</Button>}
    {error && <p role="alert" className="chat-media-error">{error}</p>}
  </section> : null;
  return <>
    <input ref={picker} type="file" multiple accept="text/*,.md,.json,.csv,.log,.yaml,.yml,image/png,image/jpeg,image/webp" hidden onChange={event => { void add(Array.from(event.target.files ?? [])); event.target.value = ''; }} />
    {children({ contextTrigger, attachmentTrigger, attachments })}
  </>;
}

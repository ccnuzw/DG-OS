import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Button, Checkbox, Input, Loading, Popover, cn } from '@dgos/ui';
import { Globe } from 'lucide-react';
import { t, useLocale } from '../../i18n';
import { noWebAccess, useChatStore, type ChatWebStatus } from '../../stores/chatStore';
import { useDevelopment } from '../../stores/developmentStore';
import { ChatNotice } from './ChatNotice';

export default function WebControls({ sessionId, disabled }: { sessionId: string; disabled: boolean }) {
  useLocale();
  const account = useDevelopment(state => state.epoch);
  const access = useChatStore(state => state.sessionWebAccess[sessionId]);
  const setAccess = useChatStore(state => state.setWebAccess);
  const statusRequest = useChatStore(state => state.webStatus);
  const configure = useChatStore(state => state.configureWeb);
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<ChatWebStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [editing, setEditing] = useState(false);
  const [apiKey, setApiKey] = useState('');
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  useLayoutEffect(() => { revision.current++; setOpen(false); setStatus(null); setApiKey(''); setEditing(false); setLoading(false); setSaving(false); setError(null); }, [account, sessionId]);
  useEffect(() => () => { revision.current++; }, []);
  const current = (ticket: number) => ticket === revision.current && useDevelopment.getState().epoch === account && useChatStore.getState().activeSessionId === sessionId;
  const load = async () => {
    const ticket = ++revision.current;
    setLoading(true); setError(null);
    try { const next = await statusRequest(sessionId); if (current(ticket)) { setStatus(next); if (next && !next.search_configured && access?.search) setAccess(sessionId, { ...access, search: false }); } }
    catch { if (current(ticket)) { setStatus(null); setError(t('chat.webStatusFailed')); } }
    finally { if (current(ticket)) setLoading(false); }
  };
  const save = async (remove = false) => {
    if (disabled || saving || (!remove && !apiKey.trim())) return;
    const ticket = ++revision.current;
    const secret = apiKey.trim();
    setApiKey(''); setSaving(true); setError(null);
    try {
      const next = await configure(sessionId, remove ? undefined : secret, remove);
      if (!current(ticket)) return;
      if (!next) { setError(t('chat.webConfigureFailed')); return; }
      setStatus(next); setEditing(false);
      if (!next.search_configured) setAccess(sessionId, { ...(access ?? noWebAccess()), search: false });
    } catch { if (current(ticket)) setError(t('chat.webConfigureFailed')); }
    finally { if (current(ticket)) setSaving(false); }
  };
  const enabled = !!access?.search || !!access?.fetch;
  return <Popover label={t('chat.webTitle')} side="top" align="start" open={open} onOpenChange={next => {
    setOpen(next); setApiKey(''); setEditing(false); setError(null);
    if (next) void load(); else { revision.current++; setLoading(false); setSaving(false); }
  }} trigger={<Button type="button" size="sm" variant="ghost" disabled={disabled} aria-label={t(enabled ? 'chat.webEnabled' : 'chat.webDisabled')} className={cn('chat-web-trigger', enabled && 'chat-composer-selected')}><Globe size={16} aria-hidden="true" /><span>{t('chat.webTitle')}</span></Button>} content={<section className="chat-web-menu">
    <header className="chat-picker-heading"><strong>{t('chat.webTitle')}</strong><span className="text-xs text-ink-secondary">{t('chat.webBackend')}</span></header>
    <p className="chat-picker-help">{t('chat.webScopeHelp')}</p>
    {loading ? <Loading label={t('chat.webChecking')} /> : null}
    {error ? <ChatNotice tone="error">{error}</ChatNotice> : null}
    <Checkbox label={t('chat.webSearchToggle')} checked={access?.search ?? false} disabled={disabled || loading || saving || !status?.search_configured} onChange={search => setAccess(sessionId, { ...(access ?? noWebAccess()), search })} />
    <Checkbox label={t('chat.webFetchToggle')} checked={access?.fetch ?? false} disabled={disabled || loading || saving || !status?.fetch_available} onChange={fetch => setAccess(sessionId, { ...(access ?? noWebAccess()), fetch })} />
    <p className="chat-picker-help">{t('chat.webPrivacyHelp')}</p>
    <p className="text-xs text-ink-secondary">{status ? t(status.search_configured ? 'chat.webConfigured' : 'chat.webUnconfigured') : t('chat.webUnknownStatus')}</p>
    {editing ? <div className="chat-web-credential">
      <Input type="password" autoComplete="new-password" spellCheck={false} label={t('chat.webApiKey')} value={apiKey} disabled={saving || disabled} onChange={event => setApiKey(event.target.value)} />
      <p className="chat-picker-help">{t('chat.webCredentialHelp')}</p>
      <div className="flex flex-wrap gap-2"><Button type="button" size="sm" loading={saving} disabled={disabled || !apiKey.trim()} onClick={() => void save()}>{t('chat.webSaveKey')}</Button><Button type="button" size="sm" variant="secondary" onClick={() => { setEditing(false); setApiKey(''); }}>{t('chat.cancel')}</Button></div>
    </div> : <div className="chat-picker-footer"><Button type="button" size="sm" variant="secondary" disabled={disabled || saving} onClick={() => setEditing(true)}>{t(status?.search_configured ? 'chat.webReplaceKey' : 'chat.webConfigure')}</Button>{status?.search_configured ? <Button type="button" size="sm" variant="ghost" disabled={disabled || saving} loading={saving} onClick={() => void save(true)}>{t('chat.webRemoveKey')}</Button> : null}<Button type="button" size="sm" variant="ghost" disabled={loading || saving} onClick={() => void load()}>{t('chat.refreshTools')}</Button></div>}
  </section>} />;
}

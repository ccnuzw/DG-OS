import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, Select } from '@dgos/ui';
import { t } from '../../i18n';
import { useDevelopment } from '../../stores/developmentStore';
import type { NativeAuthKind, NativeAuthRequest, NativeAuthSnapshot } from '../../types/nativeAuth';
import { nativeError } from '../../stores/nativeExtensions';
import { useNativeManagerGuard } from './NativeManagerLifecycle';

// Only operation identity survives a remount. URLs and callback values never enter a store.
const requests = new Map<string, NativeAuthRequest>();
const unresolved = new Set<string>();
export function hasPendingNativeAuthentication(sessionKey: string): boolean { return unresolved.has(sessionKey); }
const listeners = new Set<() => void>();
let revision = 0;
const notify = () => { revision++; listeners.forEach(listener => listener()); };
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
useDevelopment.subscribe((next, previous) => { if (next.epoch !== previous.epoch) { requests.clear(); unresolved.clear(); notify(); } });
const ended = (state: string) => ['succeeded', 'failed', 'stopped', 'auth_unknown'].includes(state);

export default function NativeAuthPanel({ session, servers, disabled }: {
  session: { session_id: string; generation: string };
  servers: { name: string; allowed: boolean }[];
  disabled: boolean;
}) {
  const epoch = useDevelopment(state => state.epoch);
  const authRevision = useSyncExternalStore(subscribe, () => revision);
  const key = JSON.stringify([epoch, session.session_id, session.generation]);
  const request = requests.get(key);
  const [server, setServer] = useState('');
  const [snapshot, setSnapshot] = useState<NativeAuthSnapshot | null>(null);
  const [callback, setCallback] = useState('');
  const [message, setMessage] = useState('');
  const [busy, setBusy] = useState(false);
  const current = useRef(key); current.current = key;
  const interactionRevision = useRef(0);
  const selected = servers.some(item => item.allowed && item.name === server) ? server : servers.find(item => item.allowed)?.name ?? '';
  const accept = (value: NativeAuthSnapshot, input: NativeAuthRequest) => {
    if (value.session_id !== input.session_id || value.generation !== input.generation || value.request_id !== input.request_id) throw new Error('mismatched auth operation');
    if (current.current !== key || useDevelopment.getState().epoch !== epoch) return;
    if (requests.get(key)?.request_id !== input.request_id) return;
    if (ended(value.state) && value.state !== 'auth_unknown') unresolved.delete(key);
    setSnapshot(value); if (!value.callback_required) setCallback('');
  };
  useEffect(() => { setMessage(''); setBusy(false); }, [key]);
  useEffect(() => {
    setSnapshot(null); setCallback('');
    if (!request || disabled) return;
    let active = true; let timer: number | undefined;
    const read = async () => {
      const interaction = interactionRevision.current;
      try {
        const value = await invoke<NativeAuthSnapshot>('read_native_auth', { input: request });
        if (!active || current.current !== key || interaction !== interactionRevision.current) return;
        accept(value, request);
        if (!ended(value.state)) timer = window.setTimeout(() => void read(), 600);
      } catch {
        if (active && current.current === key) { setSnapshot(null); setCallback(''); setMessage(t('nativeExtensions.auth.readFailed')); }
      }
    };
    void read();
    return () => { active = false; clearTimeout(timer); };
  }, [key, request, disabled, authRevision]);
  const start = async (kind: NativeAuthKind) => {
    if (busy || disabled || !selected || (request && (!snapshot || !ended(snapshot.state) || snapshot.state === 'auth_unknown'))) return;
    const input = { ...session, request_id: crypto.randomUUID() };
    interactionRevision.current++;
    requests.set(key, input); unresolved.add(key); notify(); setBusy(true); setSnapshot(null); setCallback(''); setMessage('');
    try { accept(await invoke<NativeAuthSnapshot>('start_native_auth', { input: { ...input, server_name: selected, kind } }), input); notify(); }
    catch (error) {
      if (current.current === key) {
        // These explicit admission refusals occur before an operation/process
        // exists. Lost responses and unclassified failures remain inspect-only.
        const refused = typeof error === 'string' && ['session_busy', 'native_extension_Unsupported', 'native_extension_InvalidFormat', 'invalid_request_id', 'request_capacity', 'maintenance_active'].includes(error);
        if (refused && useDevelopment.getState().epoch === epoch && requests.get(key) === input) { requests.delete(key); unresolved.delete(key); notify(); }
        setMessage(refused ? t(`nativeExtensions.error.${nativeError(error)}`) : t('nativeExtensions.auth.startFailed'));
      }
    }
    finally { if (current.current === key) setBusy(false); }
  };
  const control = async (command: 'cancel_native_auth' | 'submit_native_auth_callback') => {
    if (!request || busy || disabled) return;
    const input = command === 'submit_native_auth_callback' ? { ...request, callback_url: callback } : request;
    interactionRevision.current++;
    setBusy(true); setMessage('');
    if (command === 'submit_native_auth_callback') {
      setCallback('');
      setSnapshot(value => value ? { ...value, authorization_url: null, callback_required: false, state: 'starting' } : null);
    }
    if (command === 'cancel_native_auth') { setCallback(''); setSnapshot(value => value ? { ...value, authorization_url: null, callback_required: false, state: 'stopping' } : null); }
    try { accept(await invoke<NativeAuthSnapshot>(command, { input }), request); notify(); }
    catch { if (current.current === key) { setCallback(''); setSnapshot(null); setMessage(t('nativeExtensions.auth.controlFailed')); } }
    finally { if (current.current === key) setBusy(false); }
  };
  const verify = async () => {
    if (!request || busy || disabled) return;
    setBusy(true); setMessage('');
    try { accept(await invoke<NativeAuthSnapshot>('read_native_auth', { input: request }), request); notify(); }
    catch { if (current.current === key) { setSnapshot(null); setCallback(''); setMessage(t('nativeExtensions.auth.readFailed')); } }
    finally { if (current.current === key) setBusy(false); }
  };
  const running = !!request && (!snapshot || !ended(snapshot.state));
  const blocked = running || snapshot?.state === 'auth_unknown';
  useNativeManagerGuard(() => {
    if (!busy && !blocked) return true;
    setMessage(t('nativeExtensions.closeBlocked')); return false;
  });
  return <section className="flex min-w-0 flex-col gap-2 border-t border-border-subtle pt-3 text-sm" aria-label={t('nativeExtensions.auth.title')}>
    <h4 className="m-0 text-sm font-medium">{t('nativeExtensions.auth.title')}</h4>
    <p className="m-0 text-ink-secondary">{t('nativeExtensions.auth.scope')}</p>
    <div className="flex flex-wrap items-end gap-2">
      <Select label={t('nativeExtensions.auth.server')} value={selected} onChange={setServer} options={servers.filter(item => item.allowed).map(item => ({ value: item.name, label: item.name }))} />
      <Button variant="secondary" disabled={disabled || busy || blocked || !selected} onClick={() => void start('login')}>{t('nativeExtensions.auth.login')}</Button>
      <Button variant="secondary" disabled={disabled || busy || blocked || !selected} onClick={() => void start('logout')}>{t('nativeExtensions.auth.logout')}</Button>
      {request && <Button variant="secondary" disabled={disabled || busy} onClick={() => void verify()}>{t('nativeExtensions.verify')}</Button>}
      {running && <Button variant="secondary" disabled={disabled || busy} onClick={() => void control('cancel_native_auth')}>{t('nativeExtensions.auth.cancel')}</Button>}
    </div>
    {request && <p className="m-0 break-all">{t('nativeExtensions.operationId')}: {request.request_id}</p>}
    {snapshot && <div role="status" className="flex min-w-0 flex-col gap-1">
      <p className="m-0">{snapshot.server_name} · {t(`nativeExtensions.auth.state.${snapshot.state}`)}</p>
      {snapshot.reason && <p className="m-0">{t(`nativeExtensions.auth.reason.${snapshot.reason}`)}</p>}
      <p className="m-0 text-ink-secondary">{t('nativeExtensions.auth.independent')}</p>
      {snapshot.credential_outcome === 'unknown' && ended(snapshot.state) && <p className="m-0">{t('nativeExtensions.auth.credentialUnknown')}</p>}
      {!ended(snapshot.state) && <p className="m-0">{t('nativeExtensions.auth.expires')}: <time dateTime={new Date(snapshot.expires_at).toISOString()}>{new Date(snapshot.expires_at).toLocaleTimeString()}</time></p>}
    </div>}
    {snapshot?.authorization_url && !disabled && <label className="flex min-w-0 flex-col gap-1">{t('nativeExtensions.auth.authorizationUrl')}<textarea readOnly rows={3} value={snapshot.authorization_url} spellCheck={false} className="w-full min-w-0 resize-y rounded border border-border-subtle bg-surface p-2 text-ink-primary" /></label>}
    {snapshot?.callback_required && !disabled && <form className="flex min-w-0 flex-col gap-2" onSubmit={event => { event.preventDefault(); void control('submit_native_auth_callback'); }}>
      <label className="flex min-w-0 flex-col gap-1">{t('nativeExtensions.auth.callback')}<textarea value={callback} onChange={event => setCallback(event.target.value)} disabled={busy} autoComplete="off" spellCheck={false} rows={2} maxLength={16384} className="w-full min-w-0 resize-y rounded border border-border-subtle bg-surface p-2 text-ink-primary" /></label>
      <p className="m-0 text-ink-secondary">{t('nativeExtensions.auth.pasteHint')}</p>
      <Button type="submit" disabled={busy || !callback.trim()}>{t('nativeExtensions.auth.submit')}</Button>
    </form>}
    {message && <p role="alert" className="m-0">{message}</p>}
  </section>;
}

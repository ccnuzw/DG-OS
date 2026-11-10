import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useRef, useState, type FocusEvent, type KeyboardEvent } from 'react';
import { Link } from 'react-router-dom';
import { Button, Checkbox, Input, Loading, Popover, cn } from '@dgos/ui';
import { ArrowUp, Check, ShieldCheck, Square } from 'lucide-react';
import { MAX_MESSAGE_BYTES, MAX_TOOLS, useChatStore, utf8Bytes } from '../../stores/chatStore';
import { ChatNotice } from './ChatNotice';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { fetchConnectedTools, toolKey, type ConnectedTool } from './catalog';
import TargetBar from './TargetBar';
import { formatNumber } from '../../i18n';
import Attachments from './Attachments';
import VoiceCapture from './VoiceCapture';
import './conversation.css';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import ContextPanel from './ContextPanel';
import SkillAccess from './SkillAccess';
import RunInspector from './RunInspector';
import { useDevelopment } from '../../stores/developmentStore';

const LINK = cn('text-accent-default underline', FOCUS_RING);

// As in Modal, WebKit may reveal only the caret and retain an obsolete body scroll offset.
function revealComposerControl(event: FocusEvent<HTMLDivElement>) {
  const region = event.currentTarget, target = event.target;
  requestAnimationFrame(() => {
    if (!region.isConnected || document.activeElement !== target || !region.contains(target)) return;
    const bounds = region.getBoundingClientRect(), field = target.getBoundingClientRect();
    const top = bounds.top + region.clientTop + 4;
    const bottom = bounds.top + region.clientTop + region.clientHeight - 4;
    if (field.top < top || field.height > bottom - top) region.scrollTop += field.top - top;
    else if (field.bottom > bottom) region.scrollTop += field.bottom - bottom;
  });
}

interface ToolPickerProps {
  tools: ConnectedTool[] | null;
  selected: string[];
  disabled: boolean;
  onLoaded: (tools: ConnectedTool[]) => void;
  onChange: (keys: string[]) => void;
  remember: boolean;
  onRemember: (remember: boolean) => void;
}

function ToolPicker({ tools, selected, disabled, onLoaded, onChange, remember, onRemember }: ToolPickerProps) {
  usePageLocale();
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const [query, setQuery] = useState('');
  const version = useRef(0);
  const latest = useRef({ selected, onLoaded, onChange });
  latest.current = { selected, onLoaded, onChange };
  useEffect(() => () => { version.current++; }, []);

  const load = () => {
    const ticket = ++version.current;
    setLoading(true);
    setFailed(false);
    fetchConnectedTools()
      .then(list => { if (ticket !== version.current) return; const current = latest.current; current.onLoaded(list); current.onChange(current.selected.filter(key => list.some(tool => toolKey(tool) === key))); })
      .catch(() => { if (ticket !== version.current) return; latest.current.onLoaded([]); setFailed(true); })
      .finally(() => { if (ticket === version.current) setLoading(false); });
  };

  return (
    <Popover label={t('chat.permissionsTitle')} side="top" align="end" open={open} onOpenChange={next => { setOpen(next); if (next) load(); else version.current++; }} trigger={
      <Button size="sm" variant="ghost" disabled={disabled} aria-label={t('chat.toolPicker', { count: selected.length, limit: MAX_TOOLS })} title={t('chat.permissionsTitle')} className={cn('chat-composer-permission-button', selected.length > 0 && 'chat-composer-selected')}><ShieldCheck className="size-4" aria-hidden="true" /><span>{t('chat.confirmEach')}</span>{selected.length > 0 ? <span className="chat-skill-count">{selected.length}</span> : null}</Button>
    } content={
        <div className="chat-tool-menu flex flex-col gap-2 text-sm">
          <header className="chat-picker-heading"><strong>{t('chat.permissionsTitle')}</strong></header>
          <div className="chat-permission-current"><ShieldCheck size={20} aria-hidden="true" /><div><strong>{t('chat.confirmEach')}</strong><p>{t('workbenchNavigation.hostPermissions')}</p></div><Check size={16} aria-hidden="true" /></div>
          <Input label={t('chat.searchTools')} value={query} onChange={event => setQuery(event.target.value)} />
          <p className="m-0 text-xs text-ink-secondary">{t('chat.toolContext')}</p>
          <p className="m-0 text-xs text-ink-secondary">{t('chat.toolsHelp')}</p>
          <Checkbox checked={remember} disabled={disabled} onChange={onRemember} label={t('chat.rememberTools')} />
          <p className="m-0 text-xs text-ink-secondary">{t('chat.rememberToolsHelp')}</p>
          {loading ? <Loading label={t('chat.loadingTools')} /> : null}
          {failed ? <p className="m-0 text-xs text-ink-secondary">{t('chat.toolsFailed')}</p> : null}
          {!loading && !failed && tools?.length === 0 ? (
            <p className="m-0 text-xs text-ink-secondary">{t('chat.noToolsBefore')}<Link to="/mcp" state={{ capabilityEntry: { runtime: 'dgos' } }} className={LINK}>{t('chat.mcp')}</Link> {t('chat.noToolsAfter')}</p>
          ) : null}
          {tools?.filter(tool => `${tool.server_name} ${tool.tool_name} ${tool.description ?? ''}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())).map(tool => {
            const key = toolKey(tool);
            const checked = selected.includes(key);
            return (
              <Checkbox key={key} className="max-w-full items-start wrap-anywhere" checked={checked}
                disabled={disabled || (!checked && selected.length >= MAX_TOOLS)}
                onChange={on => onChange(on ? [...selected, key] : selected.filter(item => item !== key))}
                label={<span>{tool.server_name} / {tool.tool_name}<small className="mt-1 block text-xs text-ink-secondary">{tool.description || t('chat.noDescription')}</small></span>} />
            );
          })}
          {tools?.length && !tools.some(tool => `${tool.server_name} ${tool.tool_name} ${tool.description ?? ''}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())) ? <p className="chat-picker-empty">{t('chat.noMatchingTools')}</p> : null}
          <div className="chat-picker-footer"><Link to="/mcp" state={{ capabilityEntry: { runtime: 'dgos' } }} className={LINK}>{t('chat.mcp')}</Link><Button size="sm" variant="ghost" onClick={load} disabled={loading}>{t('chat.refreshTools')}</Button></div>
        </div>
    } />
  );
}

export default function Composer() {
  usePageLocale();
  const session = useChatStore(state => state.activeSession);
  const active = useSurfaceActive();
  const sessionId = session?.id ?? null;
  const stale = useChatStore(state => state.activeStale);
  const generation = useChatStore(state => state.generation);
  const actionError = useChatStore(state => state.actionError);
  const actionCode = useChatStore(state => state.actionCode);
  const draft = useChatStore(state => sessionId ? state.sessionDrafts[sessionId] ?? '' : '');
  const selectedTools = useChatStore(state => sessionId ? state.sessionTools[sessionId] ?? [] : []);
  const rememberTools = useChatStore(state => sessionId ? state.sessionRememberTools[sessionId] ?? false : false);
  const setRememberTools = useChatStore(state => state.setRememberTools);
  const unknown = useChatStore(state => !!sessionId && (state.sessionUnknown[sessionId] || state.messages.some(message => message.result_unknown || !!message.audit_warning)));
  const accountBusy = useChatStore(state => state.preparingSessionId !== null || Object.keys(state.generations).length > 0);
  const [preparing, setPreparing] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const preparingRef = useRef(false);
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const assets = useChatStore(state => sessionId ? state.sessionAssets[sessionId] : undefined);
  const hasContext = useChatStore(state => !!sessionId && !!state.sessionFoundation[sessionId]?.context_refs.length);
  const [assetBusy, setAssetBusy] = useState(false);
  const [voiceBusy, setVoiceBusy] = useState(false);
  const setDraft = useChatStore(state => state.setDraft);
  const setSessionTools = useChatStore(state => state.setSessionTools);
  const send = useChatStore(state => state.send);
  const stop = useChatStore(state => state.stop);

  const [tools, setTools] = useState<ConnectedTool[] | null>(null);
  const [pickerKey, setPickerKey] = useState(0);
  const [skillRequest, setSkillRequest] = useState(0);
  const composing = useRef(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const form = useRef<HTMLFormElement>(null);
  const wasGenerating = useRef(false);

  const generating = generation !== null;
  const hasTarget = !!session?.provider_id && !!session?.model_id;
  const bytes = utf8Bytes(draft);
  const tooLong = bytes > MAX_MESSAGE_BYTES;
  const canSend = !!session && !stale && !accountBusy && !unknown && !preparing && !assetBusy && !voiceBusy && hasTarget && (draft.trim().length > 0 || !!assets?.length || hasContext) && !tooLong;


  useEffect(() => {
    const focused = document.activeElement;
    if (active && wasGenerating.current && !generating && (focused === document.body || (focused && form.current?.contains(focused)))) input.current?.focus({ preventScroll: true });
    wasGenerating.current = generating;
  }, [active, generating]);

  useEffect(() => {
    if (actionCode !== 1006017) return;
    if (sessionId) setSessionTools(sessionId, []);
    setTools(null);
    setPickerKey(key => key + 1);
  }, [actionCode, actionError, sessionId, setSessionTools]);

  useEffect(() => { setTools(null); setPickerKey(key => key + 1); setAssetBusy(false); setVoiceBusy(false); }, [sessionId]);

  const submit = async () => {
    if (!canSend || preparingRef.current || !sessionId) return;
    const owner = sessionId;
    const account = useDevelopment.getState().epoch;
    preparingRef.current = true;
    setPreparing(true);
    setSubmitError(null);
    try {
      const available = selectedTools.length ? await fetchConnectedTools() : [];
      if (!mounted.current || useChatStore.getState().activeSessionId !== owner || useDevelopment.getState().epoch !== account) return;
      const chosen = selectedTools.map(key => available.find(tool => toolKey(tool) === key));
      if (chosen.some(tool => !tool)) { setSubmitError(t('chat.toolsChanged')); return; }
      setTools(available);
      await send(draft, chosen.filter((tool): tool is ConnectedTool => !!tool).map(tool => ({ server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash })));
    } catch { if (mounted.current && useChatStore.getState().activeSessionId === owner && useDevelopment.getState().epoch === account) setSubmitError(t('chat.toolsFailed')); }
    finally { preparingRef.current = false; if (mounted.current) setPreparing(false); }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== 'Enter') return;
    // IME candidate selection also reports Enter; it must never send (FR-006 §9.2).
    if (event.nativeEvent.isComposing || composing.current || event.keyCode === 229) return;
    if (event.shiftKey || event.ctrlKey || event.metaKey) {
      event.preventDefault();
      const area = event.currentTarget;
      const { selectionStart, selectionEnd } = area;
      if (sessionId) setDraft(sessionId, `${draft.slice(0, selectionStart)}\n${draft.slice(selectionEnd)}`);
      requestAnimationFrame(() => area.setSelectionRange(selectionStart + 1, selectionStart + 1));
      return;
    }
    event.preventDefault();
    void submit();
  };

  if (!session) return null;

  return (
    <Attachments sessionId={session.id} disabled={stale || generating || voiceBusy} onBusy={setAssetBusy}>{media =>
    <TargetBar session={session} skillRequest={skillRequest}>{target =>
    <form ref={form} aria-label={t('chat.sendForm')} className="chat-composer" onSubmit={event => { event.preventDefault(); void submit(); }}>
      <div className="chat-composer-body" onFocusCapture={revealComposerControl}>
      <div className="chat-composer-context">{target.context}{media.attachments}</div>
      {actionError ? <ChatNotice tone="error">{actionError}</ChatNotice> : null}
      {submitError ? <ChatNotice tone="error">{submitError}</ChatNotice> : null}
      {unknown ? <ChatNotice tone="warning">{t('chat.continueBlocked')}</ChatNotice> : null}
      {actionCode === 1006013 && !generating ? (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs text-ink-secondary">{t('chat.skillUnavailable')}</span>
          <Button size="sm" variant="secondary" onClick={() => setSkillRequest(value => value + 1)}>{t('chat.reselectSkill')}</Button>
        </div>
      ) : null}
      {actionCode === 1006017 ? (
        <p className="m-0 text-xs text-ink-secondary">
          {generating ? t('chat.stopFirst') : t('chat.toolsCleared')}
        </p>
      ) : null}
      {stale ? <p className="m-0 text-xs text-status-error-text">{t('chat.staleSend')}</p> : null}
      <Input
        ref={input}
        multiline
        rows={2}
        aria-label={t('chat.input')}
        aria-describedby="chat-input-hint"
        placeholder={t('chat.inputPlaceholder')}
        value={draft}
        disabled={!session || stale || generating}
        error={tooLong ? t('chat.tooLong') : undefined}
        className="type-reading chat-composer-input"
        onChange={event => { if (sessionId) setDraft(sessionId, event.target.value); }}
        onKeyDown={onKeyDown}
        onCompositionStart={() => { composing.current = true; }}
        onCompositionEnd={() => { composing.current = false; }}
      />
      <p id="chat-input-hint" className={cn('chat-composer-counter m-0 text-xs', bytes < MAX_MESSAGE_BYTES * 0.9 && 'sr-only', tooLong ? 'text-status-error-text' : 'text-ink-secondary')}>
        {formatNumber(bytes)} / {formatNumber(MAX_MESSAGE_BYTES)} {t('chat.byteUnit')}
      </p>
      </div>
      <div className="chat-composer-footer">
        <div className="chat-composer-toolbar" role="group" aria-label={t('chat.composerTools')}>
          {media.attachmentTrigger}{target.settings}{media.contextTrigger}
          <ContextPanel session={session} content={draft} toolKeys={selectedTools} disabled={stale || accountBusy || preparing} />
          <SkillAccess session={session} content={draft} disabled={stale || accountBusy || preparing} />
          <RunInspector sessionId={session.id} toolKeys={selectedTools} disabled={stale || accountBusy || preparing} />
          {sessionId ? <VoiceCapture key={`voice:${sessionId}`} sessionId={sessionId} disabled={stale || generating || assetBusy} onBusy={setVoiceBusy} /> : null}
          {target.skills}
        </div>
        <div className="chat-composer-primary" role="group" aria-label={t('chat.composerModel')}>
          <ToolPicker key={pickerKey} tools={tools} selected={selectedTools} disabled={generating || !session || stale}
            remember={rememberTools} onRemember={remember => { if (sessionId) setRememberTools(sessionId, remember); }}
            onLoaded={setTools} onChange={keys => { if (sessionId) setSessionTools(sessionId, keys); }} />
          {target.model}{target.effort}
          {generating
            ? <Button key="stop" type="button" variant="danger" size="sm" iconOnly className="chat-composer-send" aria-label={t('chat.stop')} title={t('chat.stop')} loading={generation?.stopping} onClick={event => { event.preventDefault(); event.stopPropagation(); void stop(); }}><Square className="size-4" aria-hidden="true" /></Button>
            : <Button key="send" type="submit" size="sm" iconOnly className="chat-composer-send" aria-label={t('chat.send')} title={!hasTarget ? t('chat.selectProviderModel') : t('chat.send')} disabled={!canSend} loading={preparing}><ArrowUp className="size-4" aria-hidden="true" /></Button>}
        </div>
      </div>
    </form>
    }</TargetBar>
    }</Attachments>
  );
}

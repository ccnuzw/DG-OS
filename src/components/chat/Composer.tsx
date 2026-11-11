import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useRef, useState, type FocusEvent, type KeyboardEvent } from 'react';
import { Link } from 'react-router-dom';
import { Button, Checkbox, Input, Loading, Popover, cn } from '@dgos/ui';
import { ArrowUp, Mic, MoreHorizontal, RotateCcw, Square, Wrench, X } from 'lucide-react';
import { MAX_INTENT_BYTES, MAX_MESSAGE_BYTES, MAX_TOOLS, useChatStore, utf8Bytes } from '../../stores/chatStore';
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
import { useDevelopment } from '../../stores/developmentStore';
import WebControls from './WebControls';

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
      .then(list => { if (ticket !== version.current) return; latest.current.onLoaded(list); })
      .catch(() => { if (ticket !== version.current) return; latest.current.onLoaded([]); setFailed(true); })
      .finally(() => { if (ticket === version.current) setLoading(false); });
  };
  useEffect(() => { load(); }, []);

  return <div className="chat-tool-menu flex flex-col gap-2 text-sm">
          <header className="chat-picker-heading"><strong>{t('chat.composerToolsTitle')}</strong></header>
          <p className="chat-picker-help">{t('chat.composerToolsHelp')}</p>
          <Input label={t('chat.searchTools')} value={query} onChange={event => setQuery(event.target.value)} />
          <Checkbox checked={remember} disabled={disabled} onChange={onRemember} label={t('chat.rememberTools')} />
          <p className="chat-picker-help">{t('chat.composerRememberHelp')}</p>
          {loading ? <Loading label={t('chat.loadingTools')} /> : null}
          {failed ? <p className="m-0 text-xs text-ink-secondary">{t('chat.toolsFailed')}</p> : null}
          {!loading && !failed && tools?.length === 0 ? (
            <p className="m-0 text-xs text-ink-secondary">{t('chat.noToolsBefore')}<Link to="/mcp" className={LINK}>{t('chat.mcp')}</Link> {t('chat.noToolsAfter')}</p>
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
          <div className="chat-picker-footer"><Link to="/mcp" className={LINK}>{t('chat.mcp')}</Link><Button size="sm" variant="ghost" onClick={load} disabled={loading}>{t('chat.refreshTools')}</Button></div>
        </div>;
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
  const continuation = useChatStore(state => sessionId ? state.sessionContinuation[sessionId] : undefined);
  const clearContinuation = useChatStore(state => state.clearContinuation);
  const continueRun = useChatStore(state => state.continueRun);
  const [continuationConfirmed, setContinuationConfirmed] = useState(false);
  useEffect(() => { setContinuationConfirmed(false); }, [sessionId, continuation?.taskId, draft]);
  const selectedTools = useChatStore(state => sessionId ? state.sessionTools[sessionId] ?? [] : []);
  const rememberTools = useChatStore(state => sessionId ? state.sessionRememberTools[sessionId] ?? false : false);
  const setRememberTools = useChatStore(state => state.setRememberTools);
  const unknown = useChatStore(state => !!sessionId && (continuation ? !state.replyRuns[continuation.taskId]?.run || !!state.replyRuns[continuation.taskId]?.run?.audit_warning || !!state.replyRuns[continuation.taskId]?.steps.some(step => step.kind === 'tool' && (step.result_unknown || step.status === 'dispatched' || (step.dispatched && !step.receipt))) : state.sessionUnknown[sessionId] || state.messages.some(message => message.result_unknown || !!message.audit_warning)));
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
  const [materialRequest, setMaterialRequest] = useState(0);
  const [moreOpen, setMoreOpen] = useState(false);
  const [morePage, setMorePage] = useState<'tools' | 'voice' | null>(null);
  const composing = useRef(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const form = useRef<HTMLFormElement>(null);
  const wasGenerating = useRef(false);

  const generating = generation !== null;
  const hasTarget = !!session?.provider_id && !!session?.model_id;
  const bytes = utf8Bytes(draft);
  const tooLong = bytes > (continuation ? MAX_INTENT_BYTES : MAX_MESSAGE_BYTES);
  const canSend = !!session && !stale && !accountBusy && !unknown && !preparing && !assetBusy && !voiceBusy && hasTarget && (continuation ? draft.trim().length > 0 && continuationConfirmed : draft.trim().length > 0 || !!assets?.length || hasContext) && !tooLong;


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

  useEffect(() => { setTools(null); setPickerKey(key => key + 1); setAssetBusy(false); setVoiceBusy(false); setMoreOpen(false); }, [sessionId]);

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
      const chosenTools = chosen.filter((tool): tool is ConnectedTool => !!tool).map(tool => ({ server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash }));
      if (continuation) await continueRun(continuation.taskId, draft, chosenTools);
      else await send(draft, chosenTools);
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
    <Attachments sessionId={session.id} openRequest={materialRequest} disabled={stale || accountBusy || preparing || voiceBusy || !!continuation} onBusy={setAssetBusy}>{media =>
    <TargetBar session={session} preparing={preparing} content={draft} skillRequest={skillRequest}>{target =>
    <form ref={form} aria-label={t('chat.sendForm')} className="chat-composer" onSubmit={event => { event.preventDefault(); void submit(); }}>
      <div className="chat-composer-body" onFocusCapture={revealComposerControl}>
      <div className="chat-composer-context">{continuation ? <Button type="button" size="sm" variant="ghost" className="chat-context-chip chat-continuation-chip" disabled={accountBusy || preparing} aria-label={t('chat.continueRemoveSource')} onClick={() => clearContinuation(session.id)}><RotateCcw size={14} aria-hidden="true" /><span>{t('chat.continueSource')}</span><X size={13} aria-hidden="true" /></Button> : null}{target.context}{!continuation ? media.attachments : null}{selectedTools.map(key => {
        const tool = tools?.find(item => toolKey(item) === key);
        let name = t('chat.composerToolsTitle');
        try { const parts: unknown = JSON.parse(key); if (Array.isArray(parts) && typeof parts[1] === 'string') name = parts[1]; } catch { /* Unavailable selections stay removable and cannot be sent. */ }
        return <Button key={key} type="button" size="xxs" variant="ghost" className="chat-context-chip chat-tool-chip" disabled={accountBusy || stale || preparing} title={tool ? `${tool.server_name} / ${tool.tool_name}` : name} aria-label={t('chat.composerRemoveTool', { name })} onClick={() => setSessionTools(session.id, selectedTools.filter(item => item !== key))}><Wrench size={13} aria-hidden="true" /><span>{name}</span><X size={13} aria-hidden="true" /></Button>;
      })}</div>
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
        error={tooLong ? t(continuation ? 'chat.intentTooLong' : 'chat.tooLong') : undefined}
        className="type-reading chat-composer-input"
        onChange={event => { if (sessionId) setDraft(sessionId, event.target.value); }}
        onKeyDown={onKeyDown}
        onCompositionStart={() => { composing.current = true; }}
        onCompositionEnd={() => { composing.current = false; }}
      />
      {continuation ? <div className="chat-composer-continuation"><p className="chat-picker-help">{t('chat.activityContinueHelp')}</p><Checkbox checked={continuationConfirmed} disabled={stale || accountBusy || preparing || unknown} onChange={setContinuationConfirmed} label={t('chat.activityContinueConfirm')} /></div> : null}
      <p id="chat-input-hint" className={cn('chat-composer-counter m-0 text-xs', bytes < (continuation ? MAX_INTENT_BYTES : MAX_MESSAGE_BYTES) * 0.9 && 'sr-only', tooLong ? 'text-status-error-text' : 'text-ink-secondary')}>
        {formatNumber(bytes)} / {formatNumber(continuation ? MAX_INTENT_BYTES : MAX_MESSAGE_BYTES)} {t('chat.byteUnit')}
      </p>
      </div>
      <div className="chat-composer-footer">
        <div className="chat-composer-toolbar" role="group" aria-label={t('chat.composerTools')}>
          {media.attachmentTrigger}{target.settings}{target.skills}
          <WebControls sessionId={session.id} disabled={stale || accountBusy || preparing} />
          <Popover label={t('chat.composerMore')} side="top" align="start" open={moreOpen} onOpenChange={next => { setMoreOpen(next); if (next) setMorePage(null); }} trigger={<Button type="button" size="sm" variant="ghost" iconOnly aria-label={t('chat.composerMore')} title={t('chat.composerMore')} className={cn('chat-composer-more', (selectedTools.length > 0 || voiceBusy) && 'chat-composer-selected')}><MoreHorizontal size={16} aria-hidden="true" /></Button>} content={<section className="chat-more-menu" data-page={morePage ?? 'menu'}>
            <header className="chat-picker-heading"><strong>{t('chat.composerMore')}</strong></header>
            <nav className="chat-more-navigation" aria-label={t('chat.composerMore')}>
              <Button type="button" size="sm" variant="ghost" className="chat-more-action" aria-label={t('chat.toolPicker', { count: selectedTools.length, limit: MAX_TOOLS })} aria-pressed={morePage === 'tools'} onClick={() => setMorePage('tools')}><Wrench size={16} aria-hidden="true" /><span>{t('chat.composerToolsTitle')}</span>{selectedTools.length ? <span>{selectedTools.length}</span> : null}</Button>
              <Button type="button" size="sm" variant="ghost" className="chat-more-action" aria-pressed={morePage === 'voice'} onClick={() => setMorePage('voice')}><Mic size={16} aria-hidden="true" /><span>{t('chat.composerVoice')}</span></Button>
            </nav>
            {morePage === 'tools' ? <ToolPicker key={pickerKey} tools={tools} selected={selectedTools} disabled={accountBusy || stale || preparing} remember={rememberTools} onRemember={remember => setRememberTools(session.id, remember)} onLoaded={setTools} onChange={keys => setSessionTools(session.id, keys)} /> : null}
            {morePage === 'voice' ? <VoiceCapture key={`voice:${session.id}`} sessionId={session.id} disabled={stale || accountBusy || preparing || assetBusy} onBusy={setVoiceBusy} onClose={() => setMoreOpen(false)} /> : null}
          </section>} />
        </div>
        <div className="chat-composer-primary" role="group" aria-label={t('chat.composerModel')}>
          {target.model}{target.effort}
          <ContextPanel session={session} content={draft} toolKeys={selectedTools} disabled={stale || accountBusy || preparing} onOpenMaterials={() => setMaterialRequest(value => value + 1)} onOpenSkills={() => setSkillRequest(value => value + 1)} />
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

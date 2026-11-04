import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Link } from 'react-router-dom';
import { Button, Checkbox, Input, Loading, Popover, cn } from '@dgos/ui';
import { MAX_MESSAGE_BYTES, MAX_TOOLS, useChatStore, utf8Bytes } from '../../stores/chatStore';
import { ChatNotice, FOCUS } from './ChatNotice';
import { fetchConnectedTools, toolKey, type ConnectedTool } from './catalog';
import TargetBar from './TargetBar';
import { formatNumber } from '../../i18n';
import Attachments from './Attachments';
import VoiceCapture from './VoiceCapture';
import './conversation.css';

const LINK = cn('text-primary underline', FOCUS);

interface ToolPickerProps {
  tools: ConnectedTool[] | null;
  selected: string[];
  disabled: boolean;
  onLoaded: (tools: ConnectedTool[]) => void;
  onChange: (keys: string[]) => void;
}

function ToolPicker({ tools, selected, disabled, onLoaded, onChange }: ToolPickerProps) {
  usePageLocale();
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
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
    <Popover label={t('chat.tools')} side="top" align="start" open={open} onOpenChange={next => { setOpen(next); if (next) load(); else version.current++; }} trigger={
      <Button size="sm" variant="ghost" disabled={disabled}>{t('chat.toolPicker', { count: selected.length, limit: MAX_TOOLS })}</Button>
    } content={
        <div className="chat-tool-menu flex flex-col gap-2 text-sm">
          <p className="m-0 text-xs text-muted-foreground">{t('chat.toolContext')}</p>
          <p className="m-0 text-xs text-muted-foreground">{t('chat.toolsHelp')}</p>
          {loading ? <Loading label={t('chat.loadingTools')} /> : null}
          {failed ? <p className="m-0 text-xs text-muted-foreground">{t('chat.toolsFailed')}</p> : null}
          {!loading && !failed && tools?.length === 0 ? (
            <p className="m-0 text-xs text-muted-foreground">{t('chat.noToolsBefore')}<Link to="/mcp" className={LINK}>{t('chat.mcp')}</Link> {t('chat.noToolsAfter')}</p>
          ) : null}
          {tools?.map(tool => {
            const key = toolKey(tool);
            const checked = selected.includes(key);
            return (
              <Checkbox key={key} className="max-w-full items-start wrap-anywhere" checked={checked}
                disabled={disabled || (!checked && selected.length >= MAX_TOOLS)}
                onChange={on => onChange(on ? [...selected, key] : selected.filter(item => item !== key))}
                label={<span>{tool.server_name} / {tool.tool_name}<small className="mt-1 block text-xs text-muted-foreground">{tool.description || t('chat.noDescription')}</small></span>} />
            );
          })}
          {tools?.length ? <Button size="sm" variant="ghost" onClick={load} disabled={loading}>{t('chat.refreshTools')}</Button> : null}
        </div>
    } />
  );
}

export default function Composer() {
  usePageLocale();
  const session = useChatStore(state => state.activeSession);
  const sessionId = session?.id ?? null;
  const stale = useChatStore(state => state.activeStale);
  const generation = useChatStore(state => state.generation);
  const actionError = useChatStore(state => state.actionError);
  const actionCode = useChatStore(state => state.actionCode);
  const lastSent = useChatStore(state => state.lastSent);
  const draft = useChatStore(state => sessionId ? state.sessionDrafts[sessionId] ?? '' : '');
  const selectedTools = useChatStore(state => sessionId ? state.sessionTools[sessionId] ?? [] : []);
  const assets = useChatStore(state => sessionId ? state.sessionAssets[sessionId] : undefined);
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
  const seenSent = useRef(lastSent.seq);
  const wasGenerating = useRef(false);

  const generating = generation !== null;
  const hasTarget = !!session?.provider_id && !!session?.model_id;
  // A session without a pinned skill and without an explicit "no skill" is refused by the host (1006013).
  const skillDecided = !!session && (session.skills.length > 0 || session.no_skill_confirmed);
  const bytes = utf8Bytes(draft);
  const tooLong = bytes > MAX_MESSAGE_BYTES;
  const canSend = !!session && !stale && !generating && !assetBusy && !voiceBusy && hasTarget && skillDecided && (draft.trim().length > 0 || !!assets?.length) && !tooLong;

  useEffect(() => {
    if (lastSent.seq === seenSent.current) return;
    seenSent.current = lastSent.seq;
    if (sessionId) { setDraft(sessionId, ''); setSessionTools(sessionId, []); }
  }, [lastSent, sessionId, setDraft, setSessionTools]);

  useEffect(() => {
    if (wasGenerating.current && !generating) input.current?.focus();
    wasGenerating.current = generating;
  }, [generating]);

  useEffect(() => {
    if (actionCode !== 1006017) return;
    if (sessionId) setSessionTools(sessionId, []);
    setTools(null);
    setPickerKey(key => key + 1);
  }, [actionCode, actionError, sessionId, setSessionTools]);

  useEffect(() => { setTools(null); setPickerKey(key => key + 1); setAssetBusy(false); setVoiceBusy(false); }, [sessionId]);

  const submit = () => {
    if (!canSend) return;
    const chosen = selectedTools
      .map(key => tools?.find(tool => toolKey(tool) === key))
      .filter((tool): tool is ConnectedTool => !!tool);
    void send(draft, chosen.map(tool => ({ server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash })));
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
    submit();
  };

  return (
    <form aria-label={t('chat.sendForm')} className="chat-composer" onSubmit={event => { event.preventDefault(); submit(); }}>
      {actionError ? <ChatNotice tone="error">{actionError}</ChatNotice> : null}
      {actionCode === 1006013 && !generating ? (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs text-muted-foreground">{t('chat.skillUnavailable')}</span>
          <Button size="sm" variant="secondary" onClick={() => setSkillRequest(value => value + 1)}>{t('chat.reselectSkill')}</Button>
        </div>
      ) : null}
      {actionCode === 1006017 ? (
        <p className="m-0 text-xs text-muted-foreground">
          {generating ? t('chat.stopFirst') : t('chat.toolsCleared')}
        </p>
      ) : null}
      {session && !hasTarget && !stale ? <p className="m-0 text-xs text-muted-foreground">{t('chat.chooseTarget')}</p> : null}
      {session && !stale && !skillDecided ? (
        <div className="flex flex-wrap items-center gap-2">
          <p id="chat-skill-hint" className="m-0 text-xs text-warning-text">
            {t('chat.chooseSkillHelp')}</p>
          <Button size="sm" variant="secondary" onClick={() => setSkillRequest(value => value + 1)}>{t('chat.chooseSkill')}</Button>
        </div>
      ) : null}
      {stale ? <p className="m-0 text-xs text-error-text">{t('chat.staleSend')}</p> : null}
      <Input
        ref={input}
        multiline
        rows={3}
        aria-label={t('chat.input')}
        aria-describedby="chat-input-hint"
        placeholder={t('chat.inputPlaceholder')}
        value={draft}
        disabled={!session || stale || generating}
        error={tooLong ? t('chat.tooLong') : undefined}
        className="chat-composer-input"
        onChange={event => { if (sessionId) setDraft(sessionId, event.target.value); }}
        onKeyDown={onKeyDown}
        onCompositionStart={() => { composing.current = true; }}
        onCompositionEnd={() => { composing.current = false; }}
      />
      {sessionId && <div className="chat-media-controls"><Attachments key={`assets:${sessionId}`} sessionId={sessionId} disabled={stale || generating || voiceBusy} onBusy={setAssetBusy} /><VoiceCapture key={`voice:${sessionId}`} sessionId={sessionId} disabled={stale || generating || assetBusy} onBusy={setVoiceBusy} /></div>}
      <div className="chat-composer-toolbar">
        {session ? <TargetBar session={session} skillRequest={skillRequest} /> : null}
        <ToolPicker key={pickerKey} tools={tools} selected={selectedTools} disabled={generating || !session || stale}
          onLoaded={setTools} onChange={keys => { if (sessionId) setSessionTools(sessionId, keys); }} />
        <p id="chat-input-hint" className={cn('m-0 text-xs', tooLong ? 'text-error-text' : 'text-muted-foreground')}>
          {formatNumber(bytes)} / {formatNumber(MAX_MESSAGE_BYTES)} {t('chat.byteUnit')}
        </p>
        {generating
          ? <Button variant="danger" loading={generation?.stopping} onClick={() => void stop()}>{t('chat.stop')}</Button>
          : <Button type="submit" disabled={!canSend}>{t('chat.send')}</Button>}
      </div>
    </form>
  );
}

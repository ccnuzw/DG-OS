import { t, useLocale as usePageLocale } from '../../i18n';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Link } from 'react-router-dom';
import { Button, Checkbox, Input, Loading, cn } from '@dgos/ui';
import { MAX_MESSAGE_BYTES, MAX_TOOLS, useChatStore, utf8Bytes } from '../../stores/chatStore';
import { ChatNotice, FOCUS } from './ChatNotice';
import { fetchConnectedTools, toolKey, type ConnectedTool } from './catalog';
import { SKILL_SELECT_ID } from './TargetBar';
import { formatNumber } from '../../i18n';

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

  const load = () => {
    setLoading(true);
    setFailed(false);
    fetchConnectedTools()
      .then(list => { onLoaded(list); onChange(selected.filter(key => list.some(tool => toolKey(tool) === key))); })
      .catch(() => { onLoaded([]); onChange([]); setFailed(true); })
      .finally(() => setLoading(false));
  };

  return (
    <section aria-label={t('chat.tools')} className="rounded-md border border-border bg-background p-2">
      <p className="m-0 mb-1 text-xs text-muted-foreground">{t('chat.toolContext')}</p>
      <Button size="sm" variant="ghost" aria-expanded={open} disabled={disabled}
        onClick={() => { const next = !open; setOpen(next); if (next && tools === null) load(); }}>
        {open ? t('chat.collapse') : t('chat.expand')} {t('chat.toolPicker', { count: selected.length, limit: MAX_TOOLS })}
      </Button>
      {open ? (
        <div className="mt-2 flex flex-col gap-2 text-sm">
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
      ) : null}
    </section>
  );
}

export default function Composer() {
  usePageLocale();
  const session = useChatStore(state => state.activeSession);
  const stale = useChatStore(state => state.activeStale);
  const generation = useChatStore(state => state.generation);
  const actionError = useChatStore(state => state.actionError);
  const actionCode = useChatStore(state => state.actionCode);
  const lastSent = useChatStore(state => state.lastSent);
  const send = useChatStore(state => state.send);
  const stop = useChatStore(state => state.stop);

  const [draft, setDraft] = useState('');
  const [tools, setTools] = useState<ConnectedTool[] | null>(null);
  const [selectedTools, setSelectedTools] = useState<string[]>([]);
  const [pickerKey, setPickerKey] = useState(0);
  const composing = useRef(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const seenSent = useRef(lastSent.seq);
  const wasGenerating = useRef(false);

  const generating = generation !== null;
  const hasTarget = !!session?.provider_id && !!session?.model_id;
  // A session without a pinned skill and without an explicit "no skill" is refused by the host (1006013).
  const skillDecided = !!session && (!!session.skill_id || session.no_skill_confirmed);
  const bytes = utf8Bytes(draft);
  const tooLong = bytes > MAX_MESSAGE_BYTES;
  const canSend = !!session && !stale && !generating && hasTarget && skillDecided && draft.trim().length > 0 && !tooLong;

  // The host accepted a send: only now are the draft and the per-send tool choice cleared. Selections never outlive a send.
  useEffect(() => {
    if (lastSent.seq === seenSent.current) return;
    seenSent.current = lastSent.seq;
    setDraft('');
    setSelectedTools([]);
  }, [lastSent]);

  useEffect(() => {
    if (wasGenerating.current && !generating) input.current?.focus();
    wasGenerating.current = generating;
  }, [generating]);

  // 1006017: the host no longer accepts this tool selection. Drop it and make the picker load a fresh list on next open.
  useEffect(() => {
    if (actionCode !== 1006017) return;
    setSelectedTools([]);
    setTools(null);
    setPickerKey(key => key + 1);
  }, [actionCode, actionError]);

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
      setDraft(`${draft.slice(0, selectionStart)}\n${draft.slice(selectionEnd)}`);
      requestAnimationFrame(() => area.setSelectionRange(selectionStart + 1, selectionStart + 1));
      return;
    }
    event.preventDefault();
    submit();
  };

  return (
    <form aria-label={t('chat.sendForm')} className="flex flex-col gap-2" onSubmit={event => { event.preventDefault(); submit(); }}>
      {actionError ? <ChatNotice tone="error">{actionError}</ChatNotice> : null}
      {actionCode === 1006013 && !generating ? (
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs text-muted-foreground">{t('chat.skillUnavailable')}</span>
          <Button size="sm" variant="secondary" onClick={() => document.getElementById(SKILL_SELECT_ID)?.focus()}>{t('chat.reselectSkill')}</Button>
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
          <Button size="sm" variant="secondary" onClick={() => document.getElementById(SKILL_SELECT_ID)?.focus()}>{t('chat.chooseSkill')}</Button>
        </div>
      ) : null}
      {stale ? <p className="m-0 text-xs text-error-text">{t('chat.staleSend')}</p> : null}
      <ToolPicker key={pickerKey} tools={tools} selected={selectedTools} disabled={generating || !session || stale}
        onLoaded={setTools} onChange={setSelectedTools} />
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
        className="resize-y leading-relaxed"
        onChange={event => setDraft(event.target.value)}
        onKeyDown={onKeyDown}
        onCompositionStart={() => { composing.current = true; }}
        onCompositionEnd={() => { composing.current = false; }}
      />
      <div className="flex flex-wrap items-center justify-between gap-2">
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

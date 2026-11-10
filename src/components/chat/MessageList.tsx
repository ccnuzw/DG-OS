import { t, formatNumber, formatDateTime, formatTime, useLocale as usePageLocale } from '../../i18n';
import { Bot, Check, Copy, RotateCcw, UserRound } from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { Button, Loading, cn } from '@dgos/ui';
import { generationErrorText, useChatStore, type ChatMessage, type ChatToolProposal, type Generation } from '../../stores/chatStore';
import { ChatNotice } from './ChatNotice';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { COPY_TEXT, useCopyFeedback } from './clipboard';
import Markdown from './Markdown';
import { AssetPreview } from './Attachments';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import RunSteps, { chatPhaseText } from './RunSteps';

const BOTTOM_FOLLOW_THRESHOLD = 2;

const ROLE_LABEL = { user: 'chat.user', assistant: 'chat.assistant' } as const;

/** The category string of a persisted message (`provider_1001006`, `task_cancelled`, ...) as readable text. */
function messageErrorText(code: string | null): string | null {
  if (!code) return null;
  const provider = /^provider_(\d+)$/.exec(code);
  return generationErrorText(code, provider ? Number(provider[1]) : null);
}

function Badge({ tone, children }: { tone: 'warning' | 'error' | 'info'; children: string }) {
  usePageLocale();
  const classes = { warning: 'bg-status-warning-surface text-status-warning-text', error: 'bg-status-error-surface text-status-error-text', info: 'bg-status-info-surface text-status-info-text' };
  return <span className={cn('rounded-sm px-2 py-0.5 text-xs font-medium', classes[tone])}>{children}</span>;
}

function MessageItem({ message, regenerateId, dropped }: { message: ChatMessage; regenerateId: string | null; dropped: number }) {
  usePageLocale();
  const regenerate = useChatStore(state => state.regenerate);
  const evidence = useChatStore(state => state.terminalEvidence[message.id]);
  const [copyState, copy] = useCopyFeedback();
  const assistant = message.role === 'assistant';
  const errorText = message.status === 'completed' ? null : messageErrorText(message.error_code);
  const timestamp = new Date(message.created_at * 1000);
  return (
    <li className={cn('chat-message-row', assistant ? 'chat-message-row-assistant' : 'chat-message-row-user')}>
      <article aria-label={t('chat.messageAria', { role: t(ROLE_LABEL[message.role]) })} className={cn('chat-message', assistant ? 'chat-assistant-message' : 'chat-user-message')}>
        <header className="chat-message-header">
          <span className={cn('chat-message-avatar', assistant ? 'chat-message-avatar-assistant' : 'chat-message-avatar-user')} aria-hidden="true">
            {assistant ? <Bot size={17} strokeWidth={1.8} /> : <UserRound size={17} strokeWidth={1.8} />}
          </span>
          <strong>{t(ROLE_LABEL[message.role])}</strong>
          <time className="chat-message-time" dateTime={timestamp.toISOString()} title={formatDateTime(timestamp)}>{formatTime(timestamp)}</time>
          {message.status === 'interrupted' ? <Badge tone="warning">{t('chat.interrupted')}</Badge> : null}
          {message.status === 'failed' ? <Badge tone="error">{t('chat.failed')}</Badge> : null}
          {message.status === 'generating' ? <Badge tone="info">{t('chat.generating')}</Badge> : null}
        </header>
        <div className="chat-message-body">
        <div className="chat-message-content">
          {assistant
            ? (message.content ? <Markdown source={message.content} /> : <p className="m-0 text-sm text-ink-secondary">{t('chat.emptyContent')}</p>)
            : <p className="type-reading m-0 whitespace-pre-wrap wrap-anywhere">{message.content}</p>}
        </div>
        {message.assets?.map(asset => <AssetPreview key={asset.id} asset={asset} />)}
        </div>
        {assistant && dropped > 0 ? <p className="mt-2 mb-0 text-xs text-ink-secondary">{t('chat.omitted', { count: formatNumber(dropped) })}</p> : null}
        {assistant && message.finish_reason === 'length' ? <p className="mt-2 mb-0 text-xs text-status-warning-text">{t('chat.truncated')}</p> : null}
        {errorText ? <p className="mt-2 mb-0 text-xs text-status-error-text">{errorText}</p> : null}
        {assistant && message.result_unknown ? <p className="mt-2 mb-0 text-xs text-status-warning-text">{t('chat.unknownResult')}</p> : null}
        {assistant && (evidence || message.dispatched !== undefined) ? <p className="mt-2 mb-0 text-xs text-ink-secondary">{t((message.dispatched ?? evidence?.dispatched) ? 'chat.dispatched' : 'chat.notDispatched')}</p> : null}
        {assistant && (message.audit_warning || evidence?.audit_warning) ? <ChatNotice tone="warning">{t('chat.auditWarning')}</ChatNotice> : null}
        {assistant && message.status !== 'generating' ? (
          <footer className="chat-message-actions">
            <Button size="xs" variant="ghost" iconOnly className="chat-copy-button" aria-label={t('chat.copy')} title={t('chat.copy')} onClick={() => copy(message.content)} disabled={!message.content}>
              {copyState === 'copied' ? <Check size={15} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />}
            </Button>
            {regenerateId ? <Button size="xs" variant="ghost" iconOnly className="chat-regenerate-button" aria-label={t('chat.regenerate')} title={t('chat.regenerate')} onClick={() => void regenerate(regenerateId)}><RotateCcw size={15} aria-hidden="true" /></Button> : null}
            <span role="status" className={cn('chat-copy-status', copyState === 'failed' && 'text-status-error-text', copyState === 'copied' && 'text-status-success-text')}>{COPY_TEXT[copyState]}</span>
            <span className="chat-token-usage">{message.total_tokens === null ? t('chat.usageUnknown') : t('chat.usage', { count: formatNumber(message.total_tokens) })}</span>
          </footer>
        ) : null}
        {!assistant ? (
          <footer className="chat-message-actions chat-user-message-actions">
            <Button size="xs" variant="ghost" iconOnly className="chat-copy-button" aria-label={t('chat.copy')} title={t('chat.copy')} onClick={() => copy(message.content)}>
              {copyState === 'copied' ? <Check size={15} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />}
            </Button>
            {regenerateId ? <Button size="xs" variant="ghost" iconOnly className="chat-regenerate-button" aria-label={t('chat.regenerate')} title={t('chat.regenerate')} onClick={() => void regenerate(regenerateId)}><RotateCcw size={15} aria-hidden="true" /></Button> : null}
            <span role="status" className={cn('chat-copy-status', copyState === 'failed' && 'text-status-error-text', copyState === 'copied' && 'text-status-success-text')}>{COPY_TEXT[copyState]}</span>
          </footer>
        ) : null}
      </article>
    </li>
  );
}

function ToolProposalCard({ proposal, busy }: { proposal: ChatToolProposal; busy: boolean }) {
  usePageLocale();
  const decide = useChatStore(state => state.decideTool);
  const active = useSurfaceActive();
  const [now, setNow] = useState(() => Date.now());
  useLayoutEffect(() => {
    if (!active) return;
    const current = Date.now();
    const deadline = proposal.expires_at * 1000;
    setNow(current);
    if (current >= deadline) return;
    let timer: number;
    const tick = () => {
      const next = Date.now();
      setNow(next);
      if (next < deadline) timer = window.setTimeout(tick, Math.min(1000, deadline - next));
    };
    timer = window.setTimeout(tick, Math.min(1000, deadline - current));
    return () => window.clearTimeout(timer);
  }, [active, proposal.id, proposal.expires_at]);
  const remaining = Math.max(0, Math.ceil((proposal.expires_at * 1000 - now) / 1000));
  const expired = remaining === 0;
  return (
    <section aria-labelledby="chat-proposal-heading" className="chat-approval-card">
      <div className="chat-approval-body">
      <h3 id="chat-proposal-heading" tabIndex={-1} className={cn('m-0 mb-2 text-base font-semibold', FOCUS_RING)}>{t('chat.confirmTool')}</h3>
      <p className="m-0 mb-2 text-sm wrap-anywhere">
        {t('chat.modelRequests')}<strong>{proposal.server_name} / {proposal.tool_name}</strong>{t('chat.argsHelp')}</p>
      <pre tabIndex={0} aria-label={t('chat.toolArgs')} className={cn('chat-tool-arguments type-code m-0 mb-2 rounded-md bg-surface-inset p-3 whitespace-pre-wrap wrap-anywhere', FOCUS_RING)}>
        {JSON.stringify(proposal.arguments, null, 2)}
      </pre>
      </div>
      <div className="chat-approval-decisions">
      <p className="m-0 mb-2 text-xs text-ink-secondary"><span role="status">{expired ? t('chat.expired') : ''}</span>{!expired ? t('chat.remaining', { seconds: formatNumber(remaining) }) : null}</p>
      <div className="flex flex-wrap gap-2">
        <Button disabled={busy || expired} onClick={() => void decide('approve')}>{t('chat.approve')}</Button>
        <Button variant="secondary" disabled={busy || expired} onClick={() => void decide('deny')}>{t('chat.reject')}</Button>
      </div>
      </div>
    </section>
  );
}

const PHASE_TEXT: Readonly<Record<Generation['phase'], string>> = {
  sending: 'chat.sending',
  running: 'chat.running',
  awaiting_tool: 'chat.awaitingTool',
  tool_running: 'chat.toolRunning',
};

function StreamingBubble({ generation }: { generation: Generation }) {
  usePageLocale();
  const [showReasoning, setShowReasoning] = useState(false);
  const status = generation.stopping ? t('chat.stopping') : generation.hostPhase ? chatPhaseText(generation.hostPhase) : generation.toolsMode && generation.phase === 'running' ? t('chat.waitingToolsModel') : t(PHASE_TEXT[generation.phase]);
  return (
    <li>
      <article aria-label={t('chat.streamingAria')} aria-busy="true" className="chat-message chat-assistant-message">
        <header className="chat-message-header">
          <span className="chat-message-avatar chat-message-avatar-assistant" aria-hidden="true"><Bot size={17} strokeWidth={1.8} /></span>
          <strong>{t('chat.assistant')}</strong>
          <Loading size="sm" label={status} aria-hidden="true" aria-live="off" />
        </header>
        {generation.kind === 'summary' ? <p className="text-xs text-ink-secondary">{t('chat.summarizing')}</p> : null}
        {generation.activities?.length ? <details><summary>{t('chat.executionActivity')}</summary><RunSteps steps={generation.activities} /></details> : null}
        {generation.text ? <Markdown source={generation.text} /> : null}
        {generation.reasoning ? (
          <div className="mt-2">
            <Button size="sm" variant="ghost" aria-expanded={showReasoning} onClick={() => setShowReasoning(value => !value)}>{showReasoning ? t('chat.hideReasoning') : t('chat.showReasoning')}</Button>
            {showReasoning ? (
              <p aria-label={t('chat.reasoningAria')} className="m-0 mt-1 whitespace-pre-wrap rounded-md bg-surface-inset p-2 text-xs text-ink-secondary wrap-anywhere">{generation.reasoning}</p>
            ) : null}
          </div>
        ) : null}
      </article>
    </li>
  );
}

export default function MessageList() {
  usePageLocale();
  const active = useSurfaceActive();
  const messages = useChatStore(state => state.messages);
  const generation = useChatStore(state => state.generation);
  const activeId = useChatStore(state => state.activeSession?.id ?? null);
  const scrollTop = useChatStore(state => activeId ? state.sessionScrollTop[activeId] : undefined);
  const setSessionScrollTop = useChatStore(state => state.setSessionScrollTop);
  const olderCursor = useChatStore(state => state.olderCursor);
  const trimmed = useChatStore(state => state.newerTrimmed);
  const loading = useChatStore(state => state.messagesLoading);
  const error = useChatStore(state => state.messagesError);
  const stale = useChatStore(state => state.activeStale);
  const dropped = useChatStore(state => state.droppedTurns);
  const unknown = useChatStore(state => !!activeId && state.sessionUnknown[activeId]);
  const loadOlder = useChatStore(state => state.loadOlderMessages);
  const loadLatest = useChatStore(state => state.loadLatestMessages);

  const live = generation && generation.sessionId === activeId ? generation : null;
  const hidden = live?.replacesMessageId ?? null;
  // The reply that is being generated is shown by the bubble, not by a persisted placeholder.
  const visible = messages.filter(message => message.id !== hidden && message.id !== live?.assistantMessageId);
  const last = visible[visible.length - 1];
  const regenerateId = !live && !trimmed && !stale && !unknown && !messages.some(message => message.result_unknown || !!message.audit_warning) && last?.role === 'assistant' && last.status !== 'generating' ? last.id : null;
  const pairedUser = visible[visible.length - 2];

  const container = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const previousHeight = useRef<number | null>(null);
  const following = useRef(new Map<string, boolean>());
  const scrollSession = useRef<string | null>(null);
  const [awayFromBottom, setAwayFromBottom] = useState(false);
  const firstId = visible[0]?.id;

  const proposalId = live?.proposal?.id ?? null;
  const runId = live?.runId ?? null;
  const previousApproval = useRef({ active, sessionId: activeId, runId, proposalId });

  useEffect(() => {
    const previous = previousApproval.current;
    previousApproval.current = { active, sessionId: activeId, runId, proposalId };
    // Selecting an existing session or revealing a surface is not a new approval arrival.
    if (active && previous.active && previous.sessionId === activeId && proposalId !== null
      && (previous.proposalId !== proposalId || previous.runId !== runId)) {
      // Radix owns focus while a nested menu, Select or Modal is open.
      const overlays = document.querySelectorAll<HTMLElement>('[role="dialog"], [role="alertdialog"], [role="listbox"], [role="menu"][data-state="open"]');
      const overlay = overlays[overlays.length - 1];
      if (overlay) {
        // A newly running task may disable the field that owned focus inside this overlay.
        if (document.activeElement === document.body) overlay.focus({ preventScroll: true });
        return;
      }
      document.getElementById('chat-proposal-heading')?.focus({ preventScroll: true });
    }
  }, [active, activeId, runId, proposalId]);
  // Persist positions in the existing store; following is presentation state scoped to each session.
  useLayoutEffect(() => {
    if (scrollSession.current === activeId) return;
    scrollSession.current = activeId;
    previousHeight.current = null;
    stick.current = activeId ? following.current.get(activeId) ?? scrollTop === undefined : true;
    setAwayFromBottom(!stick.current);
  }, [activeId, scrollTop]);

  useLayoutEffect(() => {
    const element = container.current;
    if (!element || !active) return;
    if (previousHeight.current !== null) {
      element.scrollTop += element.scrollHeight - previousHeight.current;
      previousHeight.current = null;
    } else if (stick.current) {
      element.scrollTop = element.scrollHeight;
    } else {
      element.scrollTop = scrollTop ?? 0;
    }
  }, [active, activeId, scrollTop, firstId, visible.length, live?.text, live?.phase, live?.proposal?.id]);

  useLayoutEffect(() => {
    const element = container.current;
    if (!element || !active) return;
    const observer = new ResizeObserver(() => {
      if (stick.current) element.scrollTop = element.scrollHeight;
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [active]);

  const onScroll = () => {
    const element = container.current;
    if (!element || !active || scrollSession.current !== activeId) return;
    // Keep streaming pinned only while effectively at the bottom. A larger tolerance
    // makes small upward scrolls look stuck and lets ResizeObserver pull the reader back.
    stick.current = element.scrollHeight - element.scrollTop - element.clientHeight <= BOTTOM_FOLLOW_THRESHOLD;
    setAwayFromBottom(!stick.current);
    if (activeId) {
      following.current.set(activeId, stick.current);
      setSessionScrollTop(activeId, element.scrollTop);
    }
  };
  const backToBottom = () => {
    stick.current = true;
    setAwayFromBottom(false);
    if (activeId) following.current.set(activeId, true);
    if (container.current) container.current.scrollTop = container.current.scrollHeight;
  };

  return (
    <>
    <p role="status" className="sr-only">{active && live ? (live.stopping ? t('chat.stopping') : live.hostPhase ? chatPhaseText(live.hostPhase) : t(PHASE_TEXT[live.phase])) : ''}</p>
    <div className="chat-timeline">
    <div ref={container} onScroll={onScroll} className="chat-message-scroll" role="region" aria-label={t('chat.messages')} aria-busy={loading}>
      <div className="chat-reading-column flex flex-col gap-3">
        {olderCursor !== null ? (
          <Button variant="secondary" loading={loading} onClick={() => { previousHeight.current = container.current?.scrollHeight ?? null; void loadOlder(); }}>{t('chat.earlier')}</Button>
        ) : null}
        {loading && !visible.length ? <Loading label={t('chat.loadingMessages')} /> : null}
        {error ? (
          <div className="flex flex-col gap-2">
            <ChatNotice tone="error">{error}</ChatNotice>
            {!stale ? <Button variant="secondary" onClick={() => void loadLatest()}>{t('chat.retry')}</Button> : null}
          </div>
        ) : null}
        {!loading && !error && !visible.length && !live ? <p className="m-0 text-sm text-ink-secondary">{t('chat.noMessages')}</p> : null}
        <ol aria-label={t('chat.messages')} className="chat-message-list">
          {visible.map(message => (
            <MessageItem key={message.id} message={message}
              regenerateId={message.id === last?.id || (pairedUser?.role === 'user' && message.id === pairedUser.id) ? regenerateId : null} dropped={dropped[message.id] ?? 0} />
          ))}
          {live ? <StreamingBubble generation={live} /> : null}
        </ol>
        {trimmed ? (
          <div className="flex flex-col gap-1">
            <p className="m-0 text-xs text-ink-secondary">{t('chat.windowTrimmed')}</p>
            <Button variant="secondary" onClick={() => void loadLatest()}>{t('chat.latest')}</Button>
          </div>
        ) : null}
      </div>
    </div>
      {awayFromBottom ? <div className="chat-jump-latest"><Button size="sm" variant="secondary" onClick={backToBottom}>{t('chat.backToBottom')}</Button></div> : null}
    </div>
    {live?.proposal ? <div className="chat-approval chat-reading-column"><ToolProposalCard proposal={live.proposal} busy={live.proposalBusy || live.stopping} /></div> : null}
    </>
  );
}

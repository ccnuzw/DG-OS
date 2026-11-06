import { useCallback, useEffect, useLayoutEffect, useReducer, useRef, useState, type FormEvent, type KeyboardEvent } from 'react';
import { Button } from '@dgos/ui/components/Button';
import { cn } from '@dgos/ui/lib/cn';
import {
  LanApiError,
  isAmbiguousFailure,
  type ChatMessage,
  type ChatSession,
  type LanApi,
  type LanSessionStatus,
} from './api';
import { chatReducer, findGenerating, initialChatState, lastAssistant } from './chatState';
import { TEXTAREA } from './classes';
import { uuidV4 } from './ids';
import { MessageItem } from './MessageItem';
import { hostStoppedText, replacedText, chatErrorText, sessionEndedText } from './messages';
import { NewSessionForm } from './NewSessionForm';
import { LanStream, type SocketLike, type StreamEnd, type StreamStatus } from './stream';
import { getRemoteLocale, t } from './i18n';

export interface ChatViewProps {
  api: LanApi;
  session: LanSessionStatus;
  /** WebSocket URL; defaults to ws://<host>/api/v1/lan/stream. */
  streamUrl?: string;
  createSocket?: (url: string) => SocketLike;
  /** The host ended the session (or it expired): go back to pairing with this text. */
  onSessionEnded: (notice: string) => void;
  onLogout: () => void;
}

type Panel = 'chat' | 'list' | 'new';

interface PendingSend {
  sessionId: string;
  content: string;
  requestId: string;
}
interface PendingRegenerate {
  messageId: string;
  requestId: string;
}

const PAGE_SIZE = 50;

function defaultSocket(url: string): SocketLike {
  return new WebSocket(url);
}

function formatTime(seconds: number): string {
  return new Intl.DateTimeFormat(getRemoteLocale(), { dateStyle: 'medium', timeStyle: 'short' }).format(seconds * 1000);
}

function sessionLabel(session: ChatSession): string {
  return session.title || t('remote.newSession');
}

export function ChatView({ api, session, streamUrl, createSocket = defaultSocket, onSessionEnded, onLogout }: ChatViewProps) {
  const [state, dispatch] = useReducer(chatReducer, initialChatState);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [panel, setPanel] = useState<Panel>('chat');
  const [streamStatus, setStreamStatus] = useState<StreamStatus>('connecting');
  const [streamEnd, setStreamEnd] = useState<StreamEnd | null>(null);
  const [draft, setDraft] = useState('');
  const [sending, setSending] = useState(false);
  const [notice, setNotice] = useState('');
  const [loadingMore, setLoadingMore] = useState(false);

  const pendingSend = useRef<PendingSend | null>(null);
  const pendingRegenerate = useRef<PendingRegenerate | null>(null);
  const activeRef = useRef<string | null>(null);
  const streamRef = useRef<LanStream | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stick = useRef(true);
  const restoreHeight = useRef<number | null>(null);
  const handlers = useRef({ resync: () => {}, end: (_end: StreamEnd) => {} });

  useEffect(() => {
    activeRef.current = activeId;
  }, [activeId]);

  const messages: ChatMessage[] = activeId ? (state.messages[activeId] ?? []) : [];
  const generating = findGenerating(messages);
  const last = lastAssistant(messages);
  const activeSession = state.sessions.find((s) => s.id === activeId);

  const refreshSessions = useCallback(async () => {
    try {
      dispatch({ type: 'sessions', sessions: await api.listChatSessions() });
    } catch (failure) {
      setNotice(chatErrorText(failure));
    }
  }, [api]);

  const loadLatest = useCallback(
    async (sessionId: string) => {
      try {
        const page = await api.listMessages(sessionId, null, PAGE_SIZE);
        dispatch({ type: 'messages', sessionId, items: page.items, nextBeforeSeq: page.next_before_seq, latest: true });
      } catch (failure) {
        if (activeRef.current === sessionId) setNotice(chatErrorText(failure));
      }
    },
    [api],
  );

  // Handlers read the latest closures through a ref so the stream, created once, never goes stale.
  useEffect(() => {
    handlers.current = {
      resync: () => {
        void refreshSessions();
        if (activeRef.current) void loadLatest(activeRef.current);
      },
      end: (end) => {
        if (end.kind === 'session_lost') onSessionEnded(end.reason === null ? sessionEndedText('') : sessionEndedText(end.reason));
        else setStreamEnd(end);
      },
    };
  });

  useEffect(() => {
    const stream = new LanStream({
      url: streamUrl ?? `ws://${location.host}/api/v1/lan/stream`,
      createSocket,
      probeSession: async () => {
        try {
          await api.getSession();
          return 'alive';
        } catch (failure) {
          return failure instanceof LanApiError && failure.status === 401 ? 'gone' : 'unknown';
        }
      },
      onFrame: (frame) => {
        if (frame.type !== 'chat') return;
        dispatch({ type: 'event', event: frame.event });
        const { event } = frame;
        if (event.kind === 'done') void refreshSessions();
        if (event.kind === 'error') {
          void refreshSessions();
          if (event.regeneration_discarded || !event.message) {
            const sessionId = event.message?.session_id ?? activeRef.current;
            if (sessionId) void loadLatest(sessionId);
          }
        }
      },
      onStatus: setStreamStatus,
      onResync: () => handlers.current.resync(),
      onEnd: (end) => handlers.current.end(end),
    });
    streamRef.current = stream;
    stream.start();
    return () => {
      stream.stop();
      streamRef.current = null;
    };
  }, [api, createSocket, streamUrl, refreshSessions, loadLatest]);

  // First load: pick the newest session, or open the creation form when there is none.
  useEffect(() => {
    let cancelled = false;
    api.listChatSessions().then(
      (sessions) => {
        if (cancelled) return;
        dispatch({ type: 'sessions', sessions });
        if (sessions.length === 0) {
          setPanel('new');
          return;
        }
        const newest = [...sessions].sort((a, b) => b.updated_at - a.updated_at)[0];
        setActiveId(newest.id);
        void loadLatest(newest.id);
      },
      (failure: unknown) => {
        if (!cancelled) setNotice(chatErrorText(failure));
      },
    );
    return () => {
      cancelled = true;
    };
  }, [api, loadLatest]);

  // Keep the newest message in view unless the user scrolled up; keep position after "load earlier".
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (restoreHeight.current !== null) {
      el.scrollTop += el.scrollHeight - restoreHeight.current;
      restoreHeight.current = null;
    } else if (stick.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [messages, state.live, panel]);

  function selectSession(id: string) {
    setActiveId(id);
    setPanel('chat');
    setNotice('');
    stick.current = true;
    void loadLatest(id);
  }

  function onDraftChange(value: string) {
    setDraft(value);
    if (pendingSend.current && pendingSend.current.content !== value.trim()) pendingSend.current = null;
  }

  async function send() {
    const content = draft.trim();
    if (!activeId || !content || sending || generating) return;
    const sessionId = activeId;
    const pending = pendingSend.current;
    // A retry after a network failure reuses the id so the host can recognise the same request.
    const requestId = pending && pending.sessionId === sessionId && pending.content === content ? pending.requestId : uuidV4();
    pendingSend.current = { sessionId, content, requestId };
    setSending(true);
    setNotice('');
    try {
      const ack = await api.sendMessage(sessionId, content, requestId);
      pendingSend.current = null;
      dispatch({ type: 'sent', sessionId, content, ack, now: Math.floor(Date.now() / 1000) });
      setDraft('');
      stick.current = true;
      if (ack.replayed) void loadLatest(sessionId);
    } catch (failure) {
      if (!isAmbiguousFailure(failure)) pendingSend.current = null;
      setNotice(chatErrorText(failure));
    } finally {
      setSending(false);
    }
  }

  async function regenerate(message: ChatMessage) {
    if (sending || generating) return;
    const pending = pendingRegenerate.current;
    const requestId = pending && pending.messageId === message.id ? pending.requestId : uuidV4();
    pendingRegenerate.current = { messageId: message.id, requestId };
    setSending(true);
    setNotice('');
    try {
      const ack = await api.regenerate(message.id, requestId);
      pendingRegenerate.current = null;
      dispatch({
        type: 'regenerated',
        sessionId: message.session_id,
        messageId: message.id,
        ack,
        now: Math.floor(Date.now() / 1000),
      });
      stick.current = true;
      if (ack.replayed) void loadLatest(message.session_id);
    } catch (failure) {
      if (!isAmbiguousFailure(failure)) pendingRegenerate.current = null;
      setNotice(chatErrorText(failure));
    } finally {
      setSending(false);
    }
  }

  async function cancel(taskId: string) {
    try {
      await api.cancelTask(taskId);
    } catch (failure) {
      setNotice(chatErrorText(failure));
    }
  }

  async function loadEarlier() {
    const cursor = activeId ? state.nextBefore[activeId] : null;
    if (!activeId || cursor === null || cursor === undefined || loadingMore) return;
    const sessionId = activeId;
    setLoadingMore(true);
    try {
      const page = await api.listMessages(sessionId, cursor, PAGE_SIZE);
      restoreHeight.current = scrollRef.current?.scrollHeight ?? null;
      dispatch({ type: 'messages', sessionId, items: page.items, nextBeforeSeq: page.next_before_seq, latest: false });
    } catch (failure) {
      setNotice(chatErrorText(failure));
    } finally {
      setLoadingMore(false);
    }
  }

  function reconnect() {
    setStreamEnd(null);
    streamRef.current?.start();
  }

  function onScroll() {
    const el = scrollRef.current;
    if (el) stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  }

  function onSubmit(event: FormEvent) {
    event.preventDefault();
    void send();
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      void send();
    }
  }

  const connected = streamStatus === 'open';
  const canSend = connected && !streamEnd && !generating && !sending && !!activeId;
  const olderCursor = activeId ? state.nextBefore[activeId] : null;
  const title = panel === 'list' ? t('remote.sessionList') : panel === 'new' ? t('remote.newSession') : activeSession ? sessionLabel(activeSession) : t('remote.chat');

  return (
    <div className="remote-chat mx-auto flex w-full flex-1 flex-col overflow-hidden">
      <header className="flex flex-wrap items-center gap-2 border-b border-border-subtle px-3 py-2">
        <Button variant="ghost" size="sm" onClick={() => setPanel(panel === 'list' ? 'chat' : 'list')}>
          {panel === 'chat' ? t('remote.sessionListButton') : t('remote.back')}
        </Button>
        <h1 className="min-w-0 flex-1 truncate text-center text-lg font-semibold">{title}</h1>
        <Button variant="ghost" size="sm" onClick={() => setPanel('new')}>
          {t('remote.new')}
        </Button>
        <Button variant="ghost" size="sm" onClick={onLogout}>
          {t('remote.logout')}
        </Button>
      </header>

      {streamEnd ? (
        <div role="status" className="flex items-center justify-between gap-2 bg-status-warning-surface px-3 py-2 text-status-warning-text">
          <span>{streamEnd.kind === 'replaced' ? replacedText() : hostStoppedText()}</span>
          <Button size="sm" variant="secondary" onClick={reconnect}>
            {t('remote.reconnect')}
          </Button>
        </div>
      ) : streamStatus === 'reconnecting' ? (
        <div role="status" className="bg-status-warning-surface px-3 py-2 text-status-warning-text">
          {t('remote.reconnecting')}
        </div>
      ) : null}

      {notice ? (
        <p role="alert" className="bg-status-error-surface px-3 py-2 text-status-error-text">
          {notice}
        </p>
      ) : null}

      {panel === 'new' ? (
        <div className="flex-1 overflow-y-auto">
          <NewSessionForm
            api={api}
            onCreated={(created) => {
              dispatch({ type: 'sessionCreated', session: created });
              selectSession(created.id);
            }}
          />
        </div>
      ) : null}

      {panel === 'list' ? (
        <ul className="flex-1 overflow-y-auto" aria-label={t('remote.sessionListButton')}>
          {state.sessions.length === 0 ? <li className="p-4 text-ink-secondary">{t('remote.noSessions')}</li> : null}
          {state.sessions.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className={cn(
                  'flex w-full flex-col items-start gap-1 border-b border-border-subtle px-4 py-3 text-left enabled:hover:bg-surface-hover',
                  item.id === activeId ? 'bg-surface-selection-neutral' : '',
                )}
                onClick={() => selectSession(item.id)}
              >
                <span className="w-full truncate text-base font-medium">{sessionLabel(item)}</span>
                <span className="w-full truncate text-sm text-ink-secondary">
                  {item.model_id ?? ''} · {formatTime(item.updated_at)}
                </span>
              </button>
            </li>
          ))}
        </ul>
      ) : null}

      {panel === 'chat' ? (
        <>
          <div ref={scrollRef} onScroll={onScroll} className="remote-reading flex-1 overflow-y-auto px-3 py-4">
            {!activeId ? <p className="text-ink-secondary">{t('remote.selectSession')}</p> : null}
            {olderCursor !== null && olderCursor !== undefined ? (
              <div className="mb-4 flex justify-center">
                <Button size="sm" variant="secondary" loading={loadingMore} onClick={() => void loadEarlier()}>
                  {t('remote.loadEarlier')}
                </Button>
              </div>
            ) : null}
            <ul className="flex flex-col gap-4" role="log" aria-label={t('remote.messages')}>
              {messages.map((message) => (
                <MessageItem
                  key={message.id}
                  message={message}
                  live={state.live[message.id]}
                  failure={state.failures[message.id]}
                  canRegenerate={
                    message.id === last?.id && message.status !== 'generating' && canSend
                  }
                  onRegenerate={(target) => void regenerate(target)}
                  onCancel={(taskId) => void cancel(taskId)}
                />
              ))}
            </ul>
          </div>
          {activeId ? (
            <form className="remote-composer flex items-end gap-2 border-t border-border-subtle p-3" onSubmit={onSubmit}>
              <textarea
                className={TEXTAREA}
                rows={2}
                aria-label={t('remote.messageLabel')}
                value={draft}
                onChange={(event) => onDraftChange(event.target.value)}
                onKeyDown={onKeyDown}
                placeholder={connected ? t('remote.messagePlaceholder') : t('remote.connecting')}
              />
              <Button type="submit" size="lg" loading={sending} disabled={!canSend || !draft.trim()}>
                {t('remote.send')}
              </Button>
            </form>
          ) : null}
        </>
      ) : null}
      <footer className="border-t border-border-subtle px-3 py-1 text-xs text-ink-tertiary">
        {session.device_label} · {session.short_id}
      </footer>
    </div>
  );
}

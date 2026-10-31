import { create } from 'zustand';
import { Channel, invoke as tauriInvoke } from '@tauri-apps/api/core';
import { extractErrorCode, lookupError } from '@dgos/ui';

// FR-006 stages 3-5 chat store. Contract: docs/04-技术架构/当前版本/V1-接口契约.md §10.1.
// - Every command is invoked as invoke('<name>', { input, onEvent? }) with snake_case input keys. Identity comes from the
//   host: no user id is ever sent.
// - Each send/regenerate creates its own Channel BEFORE invoking. Events can arrive before invoke resolves, so they are
//   buffered until the SendAck names the assistant message they belong to (B06-59).
// - A terminal event carries the persisted message snapshot; the accumulated delta text is display-only (B06-60).
// - Reasoning text lives only in `generation.reasoning`. It is never written to `messages`, never copied.

export type ChatMessageStatus = 'generating' | 'completed' | 'interrupted' | 'failed';
export interface ChatSession {
  id: string; title: string; title_source: 'auto' | 'user';
  provider_id: string | null; model_id: string | null;
  skill_id: string | null; skill_version_id: string | null;
  no_skill_confirmed: boolean;
  created_at: number; updated_at: number;
}
export interface ChatMessage {
  id: string; session_id: string; seq: number;
  role: 'user' | 'assistant'; content: string; status: ChatMessageStatus;
  task_id: string | null; finish_reason: string | null;
  error_code: string | null;
  total_tokens: number | null;
  result_unknown: boolean;
  created_at: number; updated_at: number;
}
export interface ChatTargetInput {
  provider_id: string | null; model_id: string | null;
  skill_id: string | null; no_skill: boolean;
}
export interface ChatSessionPage { items: ChatSession[]; next_cursor: string | null }
export interface ChatMessagePage { items: ChatMessage[]; next_before_seq: number | null }
export interface ChatSearchHit { session: ChatSession; snippet: string; match_count: number }
export interface ChatSearchPage { items: ChatSearchHit[]; next_cursor: string | null }
export interface ChatToolSelection { server_id: string; tool_name: string; schema_hash: string }
export interface SendAck {
  task_id: string; user_message_id: string; assistant_message_id: string;
  dropped_turns: number; replayed: boolean;
}
export interface ChatToolProposal {
  id: string; server_id: string; server_name: string; tool_name: string;
  schema_hash: string; arguments: unknown; expires_at: number; // Unix seconds
}
export type ChatStreamEvent =
  | { kind: 'started'; task_id: string; assistant_message_id: string }
  | { kind: 'delta'; assistant_message_id: string; text: string }
  | { kind: 'reasoning_delta'; assistant_message_id: string; text: string }
  | { kind: 'tool_proposal'; assistant_message_id: string; proposal: ChatToolProposal }
  | { kind: 'done'; message: ChatMessage }
  | { kind: 'error'; assistant_message_id: string; message: ChatMessage | null;
      error_code: string; provider_code: number | null;
      result_unknown: boolean; regeneration_discarded: boolean };

interface CommandResponse<T> {
  success: boolean; data?: T; error?: { code?: number; message?: string; detail?: unknown };
}

export const SESSION_PAGE = 30;
export const SESSION_CAP = 300;
export const MESSAGE_PAGE = 50;
export const MESSAGE_CAP = 200;
export const MAX_MESSAGE_BYTES = 16384;
export const MAX_TOOLS = 8;
export const SEARCH_DEBOUNCE_MS = 300;
export const SEARCH_MAX_CHARS = 128;
export const TITLE_MAX_CHARS = 80;

export const utf8Bytes = (text: string): number => new TextEncoder().encode(text).length;
export const charCount = (text: string): number => Array.from(text).length;

/** A failed command. `message` is never shown: screens use `chatErrorText(code)`. */
export class ChatCommandError extends Error {
  readonly code: number | null;
  readonly detail: unknown;
  constructor(code: number | null, detail?: unknown) {
    super(code === null ? 'chat command failed' : `chat command failed (${code})`);
    this.name = 'ChatCommandError';
    this.code = code;
    this.detail = detail;
  }
}

const CHAT_ERRORS: Readonly<Record<number, string>> = Object.freeze({
  1006001: '会话不存在或已被删除。',
  1006002: '消息不存在。',
  1006005: '创建会话失败，请稍后重试。',
  1006006: '删除会话失败，请稍后重试。',
  1006007: '内容无效：消息不能为空且不能超过 16 KiB，会话标题需为 1 到 80 个字符。',
  1006008: '上下文过长，请新建会话后再试。',
  1006009: '还没有可用的 Provider，请先到 Provider 页面完成配置。',
  1006010: '已有生成正在进行，请先停止当前生成。',
  1006011: '请求标识冲突，请重新发送。',
  1006012: 'Provider 的目标地址已变化，确认后才能发送。',
  1006013: '会话固定的技能不可用，请重新选择技能或明确选择无技能。',
  1006014: '只能重新生成会话中最后一条助手回复。',
  1006015: '请先为会话选择 Provider 和模型。',
  1006016: '搜索词无效：去掉首尾空白后需为 1 到 128 个字符。',
  1006017: '工具确认已失效、已过期或与当前提案不符，请重新发送并重新选择工具。',
});

/** Fixed Chinese text for a chat error. The host's own message text is never rendered. */
export function chatErrorText(error: unknown): string {
  const code = error instanceof ChatCommandError ? error.code : extractErrorCode(error);
  if (code === null) return '操作失败，请稍后重试。';
  if (Object.prototype.hasOwnProperty.call(CHAT_ERRORS, code)) return CHAT_ERRORS[code];
  const info = lookupError(code);
  if (info) return info.description ? `${info.message}：${info.description}` : info.message;
  return `操作失败（错误码 ${code}）`;
}

/** Text for the `error_code` category of a terminal error event (D074). */
export function generationErrorText(errorCode: string, providerCode: number | null): string {
  if (errorCode === 'task_cancelled') return '已停止生成。';
  if (errorCode === 'process_interrupted') return '生成因应用退出或重启而中断。';
  if (errorCode === 'generation_limit') return '回复达到宿主的时长或大小上限，已在此处中断。';
  if (errorCode === 'empty_response') return '模型返回了空回复。';
  if (providerCode !== null) {
    const info = lookupError(providerCode);
    return info ? info.message : `模型服务返回错误（错误码 ${providerCode}）`;
  }
  if (errorCode === 'provider_timeout') return '请求超时。';
  return '生成未完成。';
}

export interface ChatChannel { onmessage: (event: ChatStreamEvent) => void }
export interface ChatDeps {
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>;
  /** Creates the Channel of one send/regenerate. Injectable like `createChannel` of the module runtime host. */
  createChannel: () => ChatChannel;
  newRequestId: () => string;
  searchDebounceMs: number;
}

export type GenerationPhase = 'sending' | 'running' | 'awaiting_tool' | 'tool_running';
export interface Generation {
  runId: string;
  sessionId: string;
  kind: 'send' | 'regenerate';
  phase: GenerationPhase;
  assistantMessageId: string | null;
  /** Display-only streamed text. The persisted snapshot replaces it at the terminal event. */
  text: string;
  /** Display-only reasoning. Never persisted, never copied. */
  reasoning: string;
  toolsMode: boolean;
  proposal: ChatToolProposal | null;
  proposalBusy: boolean;
  stopping: boolean;
  /** Regenerate: the reply being replaced, hidden while the new one is generating. */
  replacesMessageId: string | null;
}
export interface ChatNotice { tone: 'error' | 'warning' | 'info'; text: string }
export type PendingDestination =
  | { kind: 'send'; sessionId: string; content: string; tools: ChatToolSelection[]; fingerprint: string; host: string }
  | { kind: 'regenerate'; sessionId: string; messageId: string; fingerprint: string; host: string };
export type SendOutcome = 'started' | 'needs_confirmation' | 'rejected';

export interface ChatState {
  sessions: ChatSession[];
  sessionsCursor: string | null;
  sessionsCapped: boolean;
  sessionsLoading: boolean;
  sessionsLoaded: boolean;
  sessionsError: string | null;

  searchQuery: string;
  searchResults: ChatSearchHit[];
  searchCursor: string | null;
  searchCapped: boolean;
  searchLoading: boolean;
  searchError: string | null;

  activeSession: ChatSession | null;
  activeStale: boolean;
  messages: ChatMessage[];
  olderCursor: number | null;
  newerTrimmed: boolean;
  messagesLoading: boolean;
  messagesError: string | null;
  droppedTurns: Record<string, number>;

  generation: Generation | null;
  notice: ChatNotice | null;
  actionError: string | null;
  /** IPC error code behind `actionError` when the host refused a command (1006013, 1006017, ...); null for local checks. */
  actionCode: number | null;
  pendingDestination: PendingDestination | null;
  /** Incremented when a send is accepted; the composer clears its draft and tool selection on a change. */
  lastSent: { seq: number; sessionId: string };

  loadSessions: () => Promise<void>;
  loadMoreSessions: () => Promise<void>;
  setSearchQuery: (query: string) => void;
  loadMoreSearch: () => Promise<void>;
  createSession: (target?: ChatTargetInput) => Promise<ChatSession | null>;
  selectSession: (session: ChatSession) => Promise<void>;
  loadLatestMessages: () => Promise<void>;
  loadOlderMessages: () => Promise<void>;
  renameSession: (sessionId: string, title: string) => Promise<string | null>;
  deleteSession: (sessionId: string) => Promise<string | null>;
  updateTarget: (sessionId: string, target: ChatTargetInput) => Promise<string | null>;
  send: (content: string, tools?: ChatToolSelection[], acceptDestination?: string) => Promise<SendOutcome>;
  regenerate: (messageId: string, acceptDestination?: string) => Promise<SendOutcome>;
  stop: () => Promise<void>;
  cancelOnLeave: () => void;
  decideTool: (type: 'approve' | 'deny') => Promise<void>;
  confirmDestination: () => void;
  dismissDestination: (token?: PendingDestination) => void;
  dismissNotice: () => void;
  reset: () => void;
}

type Data = Omit<ChatState, 'loadSessions' | 'loadMoreSessions' | 'setSearchQuery' | 'loadMoreSearch' | 'createSession'
  | 'selectSession' | 'loadLatestMessages' | 'loadOlderMessages' | 'renameSession' | 'deleteSession' | 'updateTarget'
  | 'send' | 'regenerate' | 'stop' | 'cancelOnLeave' | 'decideTool' | 'confirmDestination' | 'dismissDestination'
  | 'dismissNotice' | 'reset'>;

const initialData = (): Data => ({
  sessions: [], sessionsCursor: null, sessionsCapped: false, sessionsLoading: false, sessionsLoaded: false, sessionsError: null,
  searchQuery: '', searchResults: [], searchCursor: null, searchCapped: false, searchLoading: false, searchError: null,
  activeSession: null, activeStale: false, messages: [], olderCursor: null, newerTrimmed: false,
  messagesLoading: false, messagesError: null, droppedTurns: {},
  generation: null, notice: null, actionError: null, actionCode: null, pendingDestination: null,
  lastSent: { seq: 0, sessionId: '' },
});

function mergeById<T>(head: T[], tail: T[], key: (item: T) => string): T[] {
  const seen = new Set(head.map(key));
  return [...head, ...tail.filter(item => !seen.has(key(item)))];
}

function upsertMessage(list: ChatMessage[], message: ChatMessage): ChatMessage[] {
  const index = list.findIndex(item => item.id === message.id);
  if (index < 0) return [...list, message];
  const copy = list.slice();
  copy[index] = message;
  return copy;
}

function eventAssistantId(event: ChatStreamEvent): string | null {
  switch (event.kind) {
    case 'done':
      return event.message?.id ?? null;
    case 'started':
      return event.assistant_message_id;
    default:
      return event.assistant_message_id ?? null;
  }
}

/** `error.detail` of destination_changed (1006012): `{ fingerprint, host }`. Anything else is not a destination change. */
function destinationDetail(detail: unknown): { fingerprint: string; host: string } | null {
  if (typeof detail !== 'object' || detail === null || !('fingerprint' in detail)) return null;
  const { fingerprint } = detail;
  if (typeof fingerprint !== 'string' || !fingerprint) return null;
  const host = 'host' in detail && typeof detail.host === 'string' ? detail.host : '';
  return { fingerprint, host };
}

interface Run {
  id: string;
  sessionId: string;
  kind: 'send' | 'regenerate';
  content: string;
  taskId: string;
  acked: boolean;
  assistantId: string | null;
  buffer: ChatStreamEvent[];
  stopRequested: boolean;
}

export function createChatStore(overrides: Partial<ChatDeps> = {}) {
  const deps: ChatDeps = {
    invoke: (command, args) => tauriInvoke(command, args),
    createChannel: () => new Channel<ChatStreamEvent>(),
    newRequestId: () => crypto.randomUUID(),
    searchDebounceMs: SEARCH_DEBOUNCE_MS,
    ...overrides,
  };

  let run: Run | null = null;
  let sessionsToken = 0;
  let messagesToken = 0;
  let searchToken = 0;
  let searchTimer: number | undefined;

  async function call<T>(command: string, args: Record<string, unknown>): Promise<T> {
    let response: CommandResponse<T>;
    try {
      response = await deps.invoke<CommandResponse<T>>(command, args);
    } catch (error) {
      throw new ChatCommandError(extractErrorCode(error));
    }
    if (!response || response.success !== true) {
      const code = response?.error?.code;
      throw new ChatCommandError(typeof code === 'number' ? code : null, response?.error?.detail);
    }
    return response.data as T;
  }

  return create<ChatState>()((set, get) => {
    const replaceSession = (session: ChatSession) => set(state => ({
      sessions: state.sessions.map(item => item.id === session.id ? session : item),
      searchResults: state.searchResults.map(hit => hit.session.id === session.id ? { ...hit, session } : hit),
      activeSession: state.activeSession?.id === session.id ? session : state.activeSession,
    }));

    const markStale = (sessionId: string) => {
      if (get().activeSession?.id === sessionId) set({ activeStale: true });
      void get().loadSessions();
    };

    /** Reloads the first page and merges it in front of what is already shown (title, order and time may have changed). */
    async function refreshSessions() {
      try {
        const page = await call<ChatSessionPage>('list_chat_sessions', { input: { limit: SESSION_PAGE } });
        set(state => {
          const merged = mergeById(page.items, state.sessions, item => item.id).slice(0, SESSION_CAP);
          const cursor = state.sessions.length ? state.sessionsCursor : page.next_cursor;
          const active = state.activeSession && merged.find(item => item.id === state.activeSession!.id);
          return {
            sessions: merged, sessionsCursor: cursor, sessionsCapped: merged.length >= SESSION_CAP && cursor !== null,
            sessionsLoaded: true, activeSession: active || state.activeSession,
          };
        });
      } catch {
        // The list is a convenience here; the terminal state of the run is already shown.
      }
    }

    function patchGeneration(r: Run, patch: Partial<Generation> | ((g: Generation) => Partial<Generation>)) {
      if (run !== r) return;
      set(state => {
        if (!state.generation || state.generation.runId !== r.id) return {};
        return { generation: { ...state.generation, ...(typeof patch === 'function' ? patch(state.generation) : patch) } };
      });
    }

    async function cancelRun(r: Run) {
      try {
        await call<boolean>('cancel_chat_generation', { taskId: r.taskId });
      } catch {
        // The terminal event (or the next list load) shows the real state; a failed cancel is never a success claim.
      }
    }

    function finalize(r: Run, snapshot: ChatMessage | null, error: Extract<ChatStreamEvent, { kind: 'error' }> | null) {
      if (run !== r) return;
      run = null;
      const generation = get().generation;
      const showing = get().activeSession?.id === r.sessionId;
      const replaced = generation?.replacesMessageId ?? null;
      let notice: ChatNotice | null = null;
      if (error) {
        const base = generationErrorText(error.error_code, error.provider_code);
        notice = error.regeneration_discarded
          ? { tone: 'warning', text: `重新生成未完成，已保留原回复。${base}` }
          : { tone: error.error_code === 'task_cancelled' ? 'info' : 'error', text: base };
        if (error.result_unknown) notice = { tone: notice.tone === 'info' ? 'warning' : notice.tone, text: `${notice.text}结果未知，上游可能已计费。` };
      }
      let reload = false;
      set(state => {
        let messages = state.messages;
        if (showing) {
          if (snapshot) {
            const keepOld = error?.regeneration_discarded === true;
            if (replaced && !keepOld && snapshot.id !== replaced) messages = messages.filter(item => item.id !== replaced);
            messages = upsertMessage(messages, snapshot);
          } else {
            reload = true;
          }
          if (messages.length > MESSAGE_CAP) reload = true;
        }
        return { generation: null, messages, notice: showing || !error ? notice : notice };
      });
      if (reload) void get().loadLatestMessages();
      void refreshSessions();
    }

    function applyEvent(r: Run, event: ChatStreamEvent) {
      if (run !== r || !event || typeof event !== 'object') return;
      const id = eventAssistantId(event);
      if (id !== null && r.assistantId !== null && id !== r.assistantId) return;
      switch (event.kind) {
        case 'started':
          return;
        case 'delta':
          return patchGeneration(r, g => ({ text: g.text + event.text }));
        case 'reasoning_delta':
          return patchGeneration(r, g => ({ reasoning: g.reasoning + event.text }));
        case 'tool_proposal':
          return patchGeneration(r, { phase: 'awaiting_tool', proposal: event.proposal, proposalBusy: false });
        case 'done':
          return finalize(r, event.message, null);
        case 'error':
          return finalize(r, event.message, event);
        default:
          return;
      }
    }

    function handleEvent(r: Run, event: ChatStreamEvent) {
      if (run !== r) return;
      if (!r.acked) {
        r.buffer.push(event);
        return;
      }
      applyEvent(r, event);
    }

    function onAck(r: Run, ack: SendAck) {
      if (run !== r) return;
      r.acked = true;
      r.assistantId = ack.assistant_message_id;
      r.taskId = ack.task_id || r.taskId;
      if (ack.replayed) {
        // The host did not call upstream again and does not re-attach the channel: show what it has stored.
        run = null;
        set({ generation: null });
        void get().loadLatestMessages();
        return;
      }
      set(state => {
        let messages = state.messages;
        if (r.kind === 'send' && state.activeSession?.id === r.sessionId && !messages.some(m => m.id === ack.user_message_id)) {
          const now = Math.floor(Date.now() / 1000);
          messages = [...messages, {
            id: ack.user_message_id, session_id: r.sessionId, seq: (messages[messages.length - 1]?.seq ?? 0) + 1,
            role: 'user', content: r.content, status: 'completed', task_id: ack.task_id, finish_reason: null,
            error_code: null, total_tokens: null, result_unknown: false, created_at: now, updated_at: now,
          }];
        }
        return {
          messages,
          droppedTurns: ack.dropped_turns > 0 ? { ...state.droppedTurns, [ack.assistant_message_id]: ack.dropped_turns } : state.droppedTurns,
          generation: state.generation && state.generation.runId === r.id
            ? { ...state.generation, assistantMessageId: ack.assistant_message_id, phase: state.generation.phase === 'sending' ? 'running' : state.generation.phase }
            : state.generation,
          lastSent: r.kind === 'send' ? { seq: state.lastSent.seq + 1, sessionId: r.sessionId } : state.lastSent,
        };
      });
      const buffered = r.buffer.splice(0);
      for (const event of buffered) applyEvent(r, event);
      if (r.stopRequested && run === r) void cancelRun(r);
    }

    async function start(
      kind: 'send' | 'regenerate', sessionId: string, command: string, input: Record<string, unknown>,
      requestId: string, content: string, toolsMode: boolean, replacesMessageId: string | null,
      pending: () => PendingDestination,
    ): Promise<SendOutcome> {
      const r: Run = {
        id: requestId, sessionId, kind, content, taskId: requestId, acked: false, assistantId: null, buffer: [], stopRequested: false,
      };
      // B06-59: the channel exists before the command is invoked, so no early event can be lost.
      const channel = deps.createChannel();
      channel.onmessage = event => handleEvent(r, event);
      run = r;
      set({
        generation: {
          runId: requestId, sessionId, kind, phase: 'sending', assistantMessageId: null, text: '', reasoning: '',
          toolsMode, proposal: null, proposalBusy: false, stopping: false, replacesMessageId,
        },
        actionError: null, actionCode: null, notice: null,
      });
      try {
        const ack = await call<SendAck>(command, { input, onEvent: channel });
        onAck(r, ack);
        return 'started';
      } catch (error) {
        if (run === r) {
          run = null;
          set({ generation: null });
        }
        const code = error instanceof ChatCommandError ? error.code : null;
        const destination = error instanceof ChatCommandError ? destinationDetail(error.detail) : null;
        if (code === 1006012 && destination) {
          const token = pending();
          set({ pendingDestination: { ...token, fingerprint: destination.fingerprint, host: destination.host } });
          return 'needs_confirmation';
        }
        set({ actionError: chatErrorText(error), actionCode: code });
        if (code === 1006001) markStale(sessionId);
        return 'rejected';
      }
    }

    return {
      ...initialData(),

      async loadSessions() {
        const token = ++sessionsToken;
        set({ sessionsLoading: true, sessionsError: null });
        try {
          const page = await call<ChatSessionPage>('list_chat_sessions', { input: { limit: SESSION_PAGE } });
          if (token !== sessionsToken) return;
          set(state => {
            const active = state.activeSession && page.items.find(item => item.id === state.activeSession!.id);
            return {
              sessions: page.items, sessionsCursor: page.next_cursor,
              sessionsCapped: page.items.length >= SESSION_CAP && page.next_cursor !== null,
              sessionsLoading: false, sessionsLoaded: true, activeSession: active || state.activeSession,
            };
          });
        } catch (error) {
          if (token === sessionsToken) set({ sessionsLoading: false, sessionsLoaded: true, sessionsError: chatErrorText(error) });
        }
      },

      async loadMoreSessions() {
        const { sessionsCursor, sessionsLoading, sessions } = get();
        if (!sessionsCursor || sessionsLoading || sessions.length >= SESSION_CAP) return;
        const token = ++sessionsToken;
        set({ sessionsLoading: true, sessionsError: null });
        try {
          const page = await call<ChatSessionPage>('list_chat_sessions', { input: { cursor: sessionsCursor, limit: SESSION_PAGE } });
          if (token !== sessionsToken) return;
          set(state => {
            const merged = mergeById(state.sessions, page.items, item => item.id).slice(0, SESSION_CAP);
            return {
              sessions: merged, sessionsCursor: page.next_cursor,
              sessionsCapped: merged.length >= SESSION_CAP && page.next_cursor !== null, sessionsLoading: false,
            };
          });
        } catch (error) {
          if (token === sessionsToken) set({ sessionsLoading: false, sessionsError: chatErrorText(error) });
        }
      },

      setSearchQuery(query) {
        clearTimeout(searchTimer);
        searchTimer = undefined;
        ++searchToken;
        const trimmed = query.trim();
        set({ searchQuery: query, searchResults: [], searchCursor: null, searchCapped: false, searchLoading: false, searchError: null });
        if (!trimmed) return;
        if (charCount(trimmed) > SEARCH_MAX_CHARS) {
          set({ searchError: chatErrorText(new ChatCommandError(1006016)) });
          return;
        }
        const token = searchToken;
        searchTimer = window.setTimeout(async () => {
          searchTimer = undefined;
          set({ searchLoading: true });
          try {
            const page = await call<ChatSearchPage>('search_chat_sessions', { input: { query: trimmed, limit: SESSION_PAGE } });
            if (token !== searchToken) return;
            set({
              searchResults: page.items, searchCursor: page.next_cursor,
              searchCapped: page.items.length >= SESSION_CAP && page.next_cursor !== null, searchLoading: false,
            });
          } catch (error) {
            if (token === searchToken) set({ searchLoading: false, searchError: chatErrorText(error) });
          }
        }, deps.searchDebounceMs);
      },

      async loadMoreSearch() {
        const { searchCursor, searchLoading, searchResults, searchQuery } = get();
        const trimmed = searchQuery.trim();
        if (!searchCursor || searchLoading || !trimmed || searchResults.length >= SESSION_CAP) return;
        const token = ++searchToken;
        set({ searchLoading: true, searchError: null });
        try {
          const page = await call<ChatSearchPage>('search_chat_sessions', { input: { query: trimmed, cursor: searchCursor, limit: SESSION_PAGE } });
          if (token !== searchToken) return;
          set(state => {
            const merged = mergeById(state.searchResults, page.items, hit => hit.session.id).slice(0, SESSION_CAP);
            return { searchResults: merged, searchCursor: page.next_cursor, searchCapped: merged.length >= SESSION_CAP && page.next_cursor !== null, searchLoading: false };
          });
        } catch (error) {
          if (token === searchToken) set({ searchLoading: false, searchError: chatErrorText(error) });
        }
      },

      async createSession(target) {
        set({ actionError: null, actionCode: null });
        try {
          const session = await call<ChatSession>('create_chat_session', { input: target ? { target } : {} });
          set(state => ({ sessions: mergeById([session], state.sessions, item => item.id).slice(0, SESSION_CAP) }));
          await get().selectSession(session);
          return session;
        } catch (error) {
          set({ actionError: chatErrorText(error), actionCode: error instanceof ChatCommandError ? error.code : null });
          return null;
        }
      },

      async selectSession(session) {
        const current = get().activeSession;
        if (current?.id === session.id && !get().activeStale) return;
        ++messagesToken;
        set({
          activeSession: session, activeStale: false, messages: [], olderCursor: null, newerTrimmed: false,
          messagesError: null, actionError: null, actionCode: null, notice: null, droppedTurns: {}, pendingDestination: null,
        });
        await get().loadLatestMessages();
      },

      async loadLatestMessages() {
        const session = get().activeSession;
        if (!session) return;
        const token = ++messagesToken;
        set({ messagesLoading: true, messagesError: null });
        try {
          const page = await call<ChatMessagePage>('list_chat_messages', { input: { session_id: session.id, limit: MESSAGE_PAGE } });
          if (token !== messagesToken) return;
          set({ messages: page.items, olderCursor: page.next_before_seq, newerTrimmed: false, messagesLoading: false });
        } catch (error) {
          if (token !== messagesToken) return;
          set({ messagesLoading: false, messagesError: chatErrorText(error) });
          if (error instanceof ChatCommandError && error.code === 1006001) markStale(session.id);
        }
      },

      async loadOlderMessages() {
        const { activeSession, olderCursor, messagesLoading } = get();
        if (!activeSession || olderCursor === null || messagesLoading) return;
        const token = ++messagesToken;
        set({ messagesLoading: true, messagesError: null });
        try {
          const page = await call<ChatMessagePage>('list_chat_messages', {
            input: { session_id: activeSession.id, before_seq: olderCursor, limit: MESSAGE_PAGE },
          });
          if (token !== messagesToken) return;
          set(state => {
            let merged = mergeById(page.items, state.messages, item => item.id);
            let trimmed = state.newerTrimmed;
            // Window cap: the side opposite to the scroll direction (the newest messages) is dropped.
            if (merged.length > MESSAGE_CAP) {
              merged = merged.slice(0, MESSAGE_CAP);
              trimmed = true;
            }
            return { messages: merged, olderCursor: page.next_before_seq, newerTrimmed: trimmed, messagesLoading: false };
          });
        } catch (error) {
          if (token === messagesToken) set({ messagesLoading: false, messagesError: chatErrorText(error) });
        }
      },

      async renameSession(sessionId, title) {
        const trimmed = title.trim();
        const length = charCount(trimmed);
        if (length < 1 || length > TITLE_MAX_CHARS) return chatErrorText(new ChatCommandError(1006007));
        try {
          const session = await call<ChatSession>('rename_chat_session', { input: { session_id: sessionId, title: trimmed } });
          replaceSession(session);
          return null;
        } catch (error) {
          if (error instanceof ChatCommandError && error.code === 1006001) markStale(sessionId);
          return chatErrorText(error);
        }
      },

      async deleteSession(sessionId) {
        try {
          await call<unknown>('delete_chat_session', { input: { session_id: sessionId } });
        } catch (error) {
          // A second delete reports "not found": the session is gone either way.
          if (!(error instanceof ChatCommandError && error.code === 1006001)) return chatErrorText(error);
        }
        ++messagesToken;
        set(state => {
          const wasActive = state.activeSession?.id === sessionId;
          return {
            sessions: state.sessions.filter(item => item.id !== sessionId),
            searchResults: state.searchResults.filter(hit => hit.session.id !== sessionId),
            ...(wasActive ? { activeSession: null, activeStale: false, messages: [], olderCursor: null, newerTrimmed: false, messagesLoading: false, messagesError: null, droppedTurns: {} } : {}),
          };
        });
        return null;
      },

      async updateTarget(sessionId, target) {
        try {
          const session = await call<ChatSession>('update_chat_session_target', { input: { session_id: sessionId, ...target } });
          replaceSession(session);
          return null;
        } catch (error) {
          if (error instanceof ChatCommandError && error.code === 1006001) markStale(sessionId);
          return chatErrorText(error);
        }
      },

      async send(content, tools = [], acceptDestination) {
        const session = get().activeSession;
        if (!session || get().activeStale || get().generation || run) return 'rejected';
        if (!content.trim()) {
          set({ actionError: '消息不能为空。', actionCode: null });
          return 'rejected';
        }
        if (utf8Bytes(content) > MAX_MESSAGE_BYTES) {
          set({ actionError: '消息超过 16 KiB 上限，请缩短后再发送。', actionCode: null });
          return 'rejected';
        }
        if (tools.length > MAX_TOOLS) {
          set({ actionError: '每次发送最多选择 8 个工具。', actionCode: null });
          return 'rejected';
        }
        // Dropping the newest side of the window means local order may be stale: re-anchor on the latest page first.
        if (get().newerTrimmed) await get().loadLatestMessages();
        if (get().activeSession?.id !== session.id || get().generation || run) return 'rejected';
        const requestId = deps.newRequestId();
        const input: Record<string, unknown> = { session_id: session.id, content, client_request_id: requestId };
        if (acceptDestination) input.accept_destination = acceptDestination;
        if (tools.length) input.tools = tools.map(tool => ({ server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash }));
        return start('send', session.id, 'send_chat_message', input, requestId, content, tools.length > 0, null,
          () => ({ kind: 'send', sessionId: session.id, content, tools, fingerprint: '', host: '' }));
      },

      async regenerate(messageId, acceptDestination) {
        const { activeSession, messages, newerTrimmed } = get();
        if (!activeSession || get().activeStale || get().generation || run) return 'rejected';
        const last = messages[messages.length - 1];
        if (newerTrimmed || !last || last.id !== messageId || last.role !== 'assistant') {
          set({ actionError: chatErrorText(new ChatCommandError(1006014)), actionCode: 1006014 });
          return 'rejected';
        }
        const requestId = deps.newRequestId();
        const input: Record<string, unknown> = { message_id: messageId, client_request_id: requestId };
        if (acceptDestination) input.accept_destination = acceptDestination;
        return start('regenerate', activeSession.id, 'regenerate_chat_message', input, requestId, '', false, messageId,
          () => ({ kind: 'regenerate', sessionId: activeSession.id, messageId, fingerprint: '', host: '' }));
      },

      async stop() {
        const r = run;
        if (!r) return;
        r.stopRequested = true;
        patchGeneration(r, { stopping: true });
        // Before the ack the host has not registered the cancel token yet: onAck cancels as soon as it can.
        if (r.acked) await cancelRun(r);
      },

      cancelOnLeave() {
        const r = run;
        if (!r) return;
        r.stopRequested = true;
        if (r.acked) void cancelRun(r);
      },

      async decideTool(type) {
        const r = run;
        const generation = get().generation;
        if (!r || !generation?.proposal || generation.proposalBusy) return;
        const proposal = generation.proposal;
        patchGeneration(r, { proposalBusy: true });
        set({ actionError: null, actionCode: null });
        try {
          await call<unknown>('advance_chat_tool_turn', { input: { task_id: r.taskId, action: { type, proposal_id: proposal.id } } });
          patchGeneration(r, { phase: 'tool_running', proposal: null, proposalBusy: false });
        } catch (error) {
          const stale = error instanceof ChatCommandError && error.code === 1006017;
          patchGeneration(r, stale ? { proposal: null, proposalBusy: false } : { proposalBusy: false });
          set({ actionError: chatErrorText(error), actionCode: error instanceof ChatCommandError ? error.code : null });
        }
      },

      confirmDestination() {
        const pending = get().pendingDestination;
        if (!pending) return;
        set({ pendingDestination: null });
        // A new request id is generated by send/regenerate: the rejected attempt never created a task row.
        if (pending.kind === 'send') void get().send(pending.content, pending.tools, pending.fingerprint);
        else void get().regenerate(pending.messageId, pending.fingerprint);
      },

      dismissDestination(token) {
        if (token && get().pendingDestination !== token) return;
        set({ pendingDestination: null });
      },

      dismissNotice() {
        set({ notice: null });
      },

      reset() {
        clearTimeout(searchTimer);
        searchTimer = undefined;
        run = null;
        ++sessionsToken;
        ++messagesToken;
        ++searchToken;
        set(initialData());
      },
    };
  });
}

export const useChatStore = createChatStore();

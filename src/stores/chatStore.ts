import { t } from '../i18n';
import { create } from 'zustand';
import { Channel, invoke as tauriInvoke } from '@tauri-apps/api/core';
import { extractErrorCode, lookupError } from '@dgos/ui';
import type { GenerationParameters } from '../components/chat/GenerationControls';

// FR-006 stages 3-5 chat store. Contract: docs/04-技术架构/当前版本/V1-接口契约.md §10.1.
// - Every command is invoked as invoke('<name>', { input, onEvent? }) with snake_case input keys. Identity comes from the
//   host: no user id is ever sent.
// - Each send/regenerate creates its own Channel BEFORE invoking. Events can arrive before invoke resolves, so they are
//   buffered until the SendAck names the assistant message they belong to (B06-59).
// - A terminal event carries the persisted message snapshot; the accumulated delta text is display-only (B06-60).
// - Reasoning text lives only in `generation.reasoning`. It is never written to `messages`, never copied.

export type ChatMessageStatus = 'generating' | 'completed' | 'interrupted' | 'failed';
export interface ChatAsset { id: string; name: string; mime: string; bytes_len: number }
export interface ChatSession {
  id: string; title: string; title_source: 'auto' | 'user';
  provider_id: string | null; model_id: string | null;
  skills: { skill_id: string; version_id: string; content_hash: string }[];
  generation: GenerationParameters;
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
  assets?: ChatAsset[];
  created_at: number; updated_at: number;
}
export interface ChatTargetInput {
  provider_id: string | null; model_id: string | null;
  skill_ids: string[]; no_skill: boolean; generation: GenerationParameters;
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

const CHAT_ERROR_CODES = new Set([1006001, 1006002, 1006005, 1006006, 1006007, 1006008, 1006009, 1006010, 1006011, 1006012, 1006013, 1006014, 1006015, 1006016, 1006017]);

/** Fixed Chinese text for a chat error. The host's own message text is never rendered. */
export function chatErrorText(error: unknown): string {
  const code = error instanceof ChatCommandError ? error.code : extractErrorCode(error);
  if (code === null) return t('chat.operationFailed');
  if (CHAT_ERROR_CODES.has(code)) return t(`chat.error${code}`);
  const info = lookupError(code);
  if (info) return info.description ? `${info.message}：${info.description}` : info.message;
  return t('chat.operationCode', { code });
}

/** Text for the `error_code` category of a terminal error event (D074). */
export function generationErrorText(errorCode: string, providerCode: number | null): string {
  if (errorCode === 'task_cancelled') return t('chat.stopped');
  if (errorCode === 'process_interrupted') return t('chat.processInterrupted');
  if (errorCode === 'generation_limit') return t('chat.generationLimit');
  if (errorCode === 'empty_response') return t('chat.emptyResponse');
  if (providerCode !== null) {
    const info = lookupError(providerCode);
    return info ? info.message : t('chat.providerCode', { code: providerCode });
  }
  if (errorCode === 'provider_timeout') return t('chat.timeout');
  return t('chat.incomplete');
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
  | { kind: 'send'; sessionId: string; content: string; tools: ChatToolSelection[]; assetIds?: string[]; fingerprint: string; host: string }
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

  /** Per-session UI state that must survive inner and outer workspace switches. */
  sessionNotices: Record<string, ChatNotice | null>;
  sessionActionErrors: Record<string, string | null>;
  sessionActionCodes: Record<string, number | null>;
  sessionPendingDestinations: Record<string, PendingDestination | null>;
  openSessionIds: string[];
  activeSessionId: string | null;
  sessionViews: Record<string, SessionViewState>;
  sessionDrafts: Record<string, string>;
  sessionTargetDrafts: Record<string, ChatTargetInput | null>;
  sessionTools: Record<string, string[]>;
  sessionAssets: Record<string, ChatAsset[]>;
  sessionScrollTop: Record<string, number>;
  generations: Record<string, Generation>;

  // Active-session projection kept stable for existing chat components and consumers.
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
  actionCode: number | null;
  pendingDestination: PendingDestination | null;
  lastSent: { seq: number; sessionId: string };

  loadSessions: () => Promise<void>;
  loadMoreSessions: () => Promise<void>;
  setSearchQuery: (query: string) => void;
  loadMoreSearch: () => Promise<void>;
  createSession: (target?: ChatTargetInput) => Promise<ChatSession | null>;
  selectSession: (session: ChatSession) => Promise<void>;
  closeSession: (sessionId: string) => void;
  loadLatestMessages: () => Promise<void>;
  loadOlderMessages: () => Promise<void>;
  setDraft: (sessionId: string, draft: string) => void;
  setSessionTargetDraft: (sessionId: string, target: ChatTargetInput | null) => void;
  setSessionTools: (sessionId: string, tools: string[]) => void;
  setSessionAssets: (sessionId: string, assets: ChatAsset[]) => void;
  setSessionScrollTop: (sessionId: string, top: number) => void;
  renameSession: (sessionId: string, title: string) => Promise<string | null>;
  deleteSession: (sessionId: string) => Promise<string | null>;
  updateTarget: (sessionId: string, target: ChatTargetInput) => Promise<string | null>;
  send: (content: string, tools?: ChatToolSelection[], acceptDestination?: string, assetIds?: string[]) => Promise<SendOutcome>;
  regenerate: (messageId: string, acceptDestination?: string) => Promise<SendOutcome>;
  stop: () => Promise<void>;
  stopSession: (sessionId: string) => Promise<void>;
  cancelAll: () => Promise<void>;
  cancelOnLeave: () => void;
  decideTool: (type: 'approve' | 'deny') => Promise<void>;
  confirmDestination: () => void;
  dismissDestination: (token?: PendingDestination) => void;
  dismissNotice: () => void;
  reset: () => void;
}

export interface SessionViewState {
  messages: ChatMessage[];
  olderCursor: number | null;
  newerTrimmed: boolean;
  messagesLoading: boolean;
  messagesError: string | null;
  droppedTurns: Record<string, number>;
  stale: boolean;
  loaded: boolean;
}

function emptySessionView(): SessionViewState {
  return { messages: [], olderCursor: null, newerTrimmed: false, messagesLoading: false, messagesError: null, droppedTurns: {}, stale: false, loaded: false };
}

function projectActive(state: Data, activeSessionId: string | null): Partial<Data> {
  const view = activeSessionId ? state.sessionViews[activeSessionId] ?? emptySessionView() : emptySessionView();
  return {
    activeSessionId,
    activeSession: activeSessionId ? state.sessions.find(session => session.id === activeSessionId) ?? null : null,
    activeStale: view.stale,
    messages: view.messages,
    olderCursor: view.olderCursor,
    newerTrimmed: view.newerTrimmed,
    messagesLoading: view.messagesLoading,
    messagesError: view.messagesError,
    droppedTurns: view.droppedTurns,
    generation: activeSessionId ? state.generations[activeSessionId] ?? null : null,
    notice: activeSessionId ? state.sessionNotices[activeSessionId] ?? null : null,
    actionError: activeSessionId ? state.sessionActionErrors[activeSessionId] ?? null : null,
    actionCode: activeSessionId ? state.sessionActionCodes[activeSessionId] ?? null : null,
    pendingDestination: activeSessionId ? state.sessionPendingDestinations[activeSessionId] ?? null : null,
  };
}

type Data = Omit<ChatState, 'loadSessions' | 'loadMoreSessions' | 'setSearchQuery' | 'loadMoreSearch' | 'createSession'
  | 'selectSession' | 'closeSession' | 'loadLatestMessages' | 'loadOlderMessages' | 'setDraft' | 'setSessionTargetDraft' | 'setSessionTools' | 'setSessionAssets' | 'setSessionScrollTop'
  | 'renameSession' | 'deleteSession' | 'updateTarget' | 'send' | 'regenerate' | 'stop' | 'stopSession' | 'cancelAll' | 'cancelOnLeave'
  | 'decideTool' | 'confirmDestination' | 'dismissDestination' | 'dismissNotice' | 'reset'>;

const initialData = (): Data => ({
  sessions: [], sessionsCursor: null, sessionsCapped: false, sessionsLoading: false, sessionsLoaded: false, sessionsError: null,
  searchQuery: '', searchResults: [], searchCursor: null, searchCapped: false, searchLoading: false, searchError: null,
  sessionNotices: {}, sessionActionErrors: {}, sessionActionCodes: {}, sessionPendingDestinations: {},
  openSessionIds: [], activeSessionId: null, sessionViews: {}, sessionDrafts: {}, sessionTargetDrafts: {}, sessionTools: {}, sessionAssets: {}, sessionScrollTop: {}, generations: {},
  activeSession: null, activeStale: false, messages: [], olderCursor: null, newerTrimmed: false,
  messagesLoading: false, messagesError: null, droppedTurns: {}, generation: null,
  notice: null, actionError: null, actionCode: null, pendingDestination: null,
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
  assets: ChatAsset[];
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

  const runs = new Map<string, Run>();
  let sessionsToken = 0;
  const messageTokens = new Map<string, number>();
  let searchToken = 0;
  let searchTimer: number | undefined;
  let accountEpoch = 0;

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
    const replaceSession = (session: ChatSession) => set(state => {
      const next = {
        ...state,
        sessions: state.sessions.map(item => item.id === session.id ? session : item),
        searchResults: state.searchResults.map(hit => hit.session.id === session.id ? { ...hit, session } : hit),
      };
      return { ...next, ...projectActive(next, state.activeSessionId) };
    });

    const markStale = (sessionId: string) => {
      set(state => {
        const view = state.sessionViews[sessionId] ?? emptySessionView();
        const next = { ...state, sessionViews: { ...state.sessionViews, [sessionId]: { ...view, stale: true } } };
        return { ...next, ...projectActive(next, state.activeSessionId) };
      });
      void get().loadSessions();
    };

    /** Reloads the first page and merges it in front of what is already shown (title, order and time may have changed). */
    async function refreshSessions() {
      const token = accountEpoch;
      try {
        const page = await call<ChatSessionPage>('list_chat_sessions', { input: { limit: SESSION_PAGE } });
        if (token !== accountEpoch) return;
        set(state => {
          const merged = mergeById(page.items, state.sessions, item => item.id).slice(0, SESSION_CAP);
          const cursor = state.sessions.length ? state.sessionsCursor : page.next_cursor;
          const next = { ...state, sessions: merged, sessionsCursor: cursor, sessionsCapped: merged.length >= SESSION_CAP && cursor !== null, sessionsLoaded: true };
          return { ...next, ...projectActive(next, state.activeSessionId) };
        });
      } catch {
        // A terminal event remains authoritative; this refresh is best-effort.
      }
    }

    function patchGeneration(r: Run, patch: Partial<Generation> | ((g: Generation) => Partial<Generation>)) {
      if (runs.get(r.sessionId) !== r) return;
      set(state => {
        const current = state.generations[r.sessionId];
        if (!current || current.runId !== r.id) return {};
        const generation = { ...current, ...(typeof patch === 'function' ? patch(current) : patch) };
        const next = { ...state, generations: { ...state.generations, [r.sessionId]: generation } };
        return { ...next, ...projectActive(next, state.activeSessionId) };
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
      if (runs.get(r.sessionId) !== r) return;
      runs.delete(r.sessionId);
      const generation = get().generations[r.sessionId];
      const replaced = generation?.replacesMessageId ?? null;
      let notice: ChatNotice | null = null;
      if (error) {
        const base = generationErrorText(error.error_code, error.provider_code);
        notice = error.regeneration_discarded ? { tone: 'warning', text: t('chat.regenerationKept', { message: base }) } : { tone: error.error_code === 'task_cancelled' ? 'info' : 'error', text: base };
        if (error.result_unknown) notice = { tone: notice.tone === 'info' ? 'warning' : notice.tone, text: `${notice.text}${t('chat.unknownResult')}` };
      }
      let reload = false;
      set(state => {
        const view = state.sessionViews[r.sessionId] ?? emptySessionView();
        let messages = view.messages;
        if (snapshot) {
          const keepOld = error?.regeneration_discarded === true;
          if (replaced && !keepOld && snapshot.id !== replaced) messages = messages.filter(item => item.id !== replaced);
          messages = upsertMessage(messages, snapshot);
        } else reload = true;
        if (messages.length > MESSAGE_CAP) reload = true;
        const next = { ...state, sessionViews: { ...state.sessionViews, [r.sessionId]: { ...view, messages } }, generations: Object.fromEntries(Object.entries(state.generations).filter(([id]) => id !== r.sessionId)), sessionNotices: { ...state.sessionNotices, [r.sessionId]: notice } };
        return { ...next, ...projectActive(next, state.activeSessionId) };
      });
      if (reload && get().activeSessionId === r.sessionId) void get().loadLatestMessages();
      void refreshSessions();
    }

    function applyEvent(r: Run, event: ChatStreamEvent) {
      if (runs.get(r.sessionId) !== r || !event || typeof event !== 'object') return;
      const id = eventAssistantId(event);
      if (id !== null && r.assistantId !== null && id !== r.assistantId) return;
      switch (event.kind) {
        case 'started': return;
        case 'delta': return patchGeneration(r, g => ({ text: g.text + event.text }));
        case 'reasoning_delta': return patchGeneration(r, g => ({ reasoning: g.reasoning + event.text }));
        case 'tool_proposal': return patchGeneration(r, { phase: 'awaiting_tool', proposal: event.proposal, proposalBusy: false });
        case 'done': return finalize(r, event.message, null);
        case 'error': return finalize(r, event.message, event);
        default: return;
      }
    }

    function handleEvent(r: Run, event: ChatStreamEvent) {
      if (runs.get(r.sessionId) !== r) return;
      if (!r.acked) { r.buffer.push(event); return; }
      applyEvent(r, event);
    }

    function onAck(r: Run, ack: SendAck) {
      if (runs.get(r.sessionId) !== r) return;
      r.acked = true;
      r.assistantId = ack.assistant_message_id;
      r.taskId = ack.task_id || r.taskId;
      if (ack.replayed) {
        runs.delete(r.sessionId);
        set(state => {
          const next = { ...state, generations: Object.fromEntries(Object.entries(state.generations).filter(([id]) => id !== r.sessionId)) };
          return { ...next, ...projectActive(next, state.activeSessionId) };
        });
        if (get().activeSessionId === r.sessionId) void get().loadLatestMessages();
        return;
      }
      set(state => {
        const view = state.sessionViews[r.sessionId] ?? emptySessionView();
        let messages = view.messages;
        if (r.kind === 'send' && !messages.some(m => m.id === ack.user_message_id)) {
          const now = Math.floor(Date.now() / 1000);
          messages = [...messages, { id: ack.user_message_id, session_id: r.sessionId, seq: (messages[messages.length - 1]?.seq ?? 0) + 1, role: 'user', content: r.content, assets: r.assets, status: 'completed', task_id: ack.task_id, finish_reason: null, error_code: null, total_tokens: null, result_unknown: false, created_at: now, updated_at: now }];
        }
        const current = state.generations[r.sessionId];
        const generation = current?.runId === r.id ? { ...current, assistantMessageId: ack.assistant_message_id, phase: current.phase === 'sending' ? 'running' as const : current.phase } : current;
        const next = { ...state, sessionViews: { ...state.sessionViews, [r.sessionId]: { ...view, messages, droppedTurns: ack.dropped_turns > 0 ? { ...view.droppedTurns, [ack.assistant_message_id]: ack.dropped_turns } : view.droppedTurns } }, generations: generation ? { ...state.generations, [r.sessionId]: generation } : state.generations, lastSent: r.kind === 'send' ? { seq: state.lastSent.seq + 1, sessionId: r.sessionId } : state.lastSent };
        return { ...next, ...projectActive(next, state.activeSessionId) };
      });
      if (r.kind === 'send') set(state => ({ sessionAssets: { ...state.sessionAssets, [r.sessionId]: (state.sessionAssets[r.sessionId] ?? []).filter(asset => !r.assets.some(sent => sent.id === asset.id)) } }));
      for (const event of r.buffer.splice(0)) applyEvent(r, event);
      if (r.stopRequested && runs.get(r.sessionId) === r) void cancelRun(r);
    }

    async function start(kind: 'send' | 'regenerate', sessionId: string, command: string, input: Record<string, unknown>, requestId: string, content: string, toolsMode: boolean, replacesMessageId: string | null, pending: () => PendingDestination): Promise<SendOutcome> {
      const epoch = accountEpoch;
      const assets = kind === 'send' ? (get().sessionAssets[sessionId] ?? []).filter(asset => (input.asset_ids as string[] | undefined)?.includes(asset.id)) : [];
      const r: Run = { id: requestId, sessionId, kind, content, assets, taskId: requestId, acked: false, assistantId: null, buffer: [], stopRequested: false };
      const channel = deps.createChannel();
      channel.onmessage = event => handleEvent(r, event);
      runs.set(sessionId, r);
      set(state => {
        const generation: Generation = { runId: requestId, sessionId, kind, phase: 'sending', assistantMessageId: null, text: '', reasoning: '', toolsMode, proposal: null, proposalBusy: false, stopping: false, replacesMessageId };
        const next = { ...state, generations: { ...state.generations, [sessionId]: generation }, sessionNotices: { ...state.sessionNotices, [sessionId]: null }, sessionActionErrors: { ...state.sessionActionErrors, [sessionId]: null }, sessionActionCodes: { ...state.sessionActionCodes, [sessionId]: null } };
        return { ...next, ...projectActive(next, state.activeSessionId) };
      });
      try {
        const ack = await call<SendAck>(command, { input, onEvent: channel });
        onAck(r, ack);
        return 'started';
      } catch (error) {
        if (epoch !== accountEpoch) return 'rejected';
        if (runs.get(sessionId) === r) {
          runs.delete(sessionId);
          set(state => {
            const next = { ...state, generations: Object.fromEntries(Object.entries(state.generations).filter(([id]) => id !== sessionId)) };
            return { ...next, ...projectActive(next, state.activeSessionId) };
          });
        }
        const code = error instanceof ChatCommandError ? error.code : null;
        const destination = error instanceof ChatCommandError ? destinationDetail(error.detail) : null;
        if (code === 1006012 && destination) {
          const token = pending();
          const pendingDestination = { ...token, fingerprint: destination.fingerprint, host: destination.host };
          set(state => ({ sessionPendingDestinations: { ...state.sessionPendingDestinations, [sessionId]: pendingDestination }, ...(state.activeSessionId === sessionId ? { pendingDestination } : {}) }));
          return 'needs_confirmation';
        }
        const text = chatErrorText(error);
        set(state => ({ sessionActionErrors: { ...state.sessionActionErrors, [sessionId]: text }, sessionActionCodes: { ...state.sessionActionCodes, [sessionId]: code }, ...(state.activeSessionId === sessionId ? { actionError: text, actionCode: code } : {}) }));
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
          set(state => { const next = { ...state, sessions: page.items, sessionsCursor: page.next_cursor, sessionsCapped: page.next_cursor !== null, sessionsLoading: false, sessionsLoaded: true }; return { ...next, ...projectActive(next, state.activeSessionId) }; });
          if (!get().activeSessionId && page.items[0]) await get().selectSession(page.items[0]);
        } catch (error) { if (token === sessionsToken) set({ sessionsLoading: false, sessionsLoaded: true, sessionsError: chatErrorText(error) }); }
      },
      async loadMoreSessions() {
        const { sessionsCursor, sessionsLoading, sessions } = get();
        if (!sessionsCursor || sessionsLoading || sessions.length >= SESSION_CAP) return;
        const token = ++sessionsToken;
        set({ sessionsLoading: true, sessionsError: null });
        try {
          const page = await call<ChatSessionPage>('list_chat_sessions', { input: { cursor: sessionsCursor, limit: SESSION_PAGE } });
          if (token !== sessionsToken) return;
          set(state => ({ sessions: mergeById(state.sessions, page.items, item => item.id).slice(0, SESSION_CAP), sessionsCursor: page.next_cursor, sessionsCapped: page.next_cursor !== null, sessionsLoading: false }));
        } catch (error) { if (token === sessionsToken) set({ sessionsLoading: false, sessionsError: chatErrorText(error) }); }
      },
      setSearchQuery(query) {
        clearTimeout(searchTimer); searchTimer = undefined; ++searchToken;
        const trimmed = query.trim();
        set({ searchQuery: query, searchResults: [], searchCursor: null, searchCapped: false, searchLoading: false, searchError: null });
        if (!trimmed) return;
        if (charCount(trimmed) > SEARCH_MAX_CHARS) { set({ searchError: chatErrorText(new ChatCommandError(1006016)) }); return; }
        const token = searchToken;
        searchTimer = window.setTimeout(async () => {
          try {
            const page = await call<ChatSearchPage>('search_chat_sessions', { input: { query: trimmed, limit: SESSION_PAGE } });
            if (token === searchToken) set({ searchResults: page.items, searchCursor: page.next_cursor, searchCapped: page.next_cursor !== null, searchLoading: false });
          } catch (error) { if (token === searchToken) set({ searchLoading: false, searchError: chatErrorText(error) }); }
        }, deps.searchDebounceMs);
      },
      async loadMoreSearch() {
        const { searchCursor, searchLoading, searchResults, searchQuery } = get();
        if (!searchCursor || searchLoading || !searchQuery.trim() || searchResults.length >= SESSION_CAP) return;
        const token = ++searchToken; set({ searchLoading: true, searchError: null });
        try {
          const page = await call<ChatSearchPage>('search_chat_sessions', { input: { query: searchQuery.trim(), cursor: searchCursor, limit: SESSION_PAGE } });
          if (token === searchToken) set(state => ({ searchResults: mergeById(state.searchResults, page.items, hit => hit.session.id).slice(0, SESSION_CAP), searchCursor: page.next_cursor, searchCapped: page.next_cursor !== null, searchLoading: false }));
        } catch (error) { if (token === searchToken) set({ searchLoading: false, searchError: chatErrorText(error) }); }
      },
      async createSession(target) {
        const token = accountEpoch;
        try {
          const session = await call<ChatSession>('create_chat_session', { input: target ? { target } : {} });
          if (token !== accountEpoch) return null;
          set(state => ({ sessions: mergeById([session], state.sessions, item => item.id).slice(0, SESSION_CAP) }));
          await get().selectSession(session); return session;
        } catch (error) { if (token === accountEpoch) set({ actionError: chatErrorText(error), actionCode: error instanceof ChatCommandError ? error.code : null }); return null; }
      },
      async selectSession(session) {
        const state = get();
        const view = state.sessionViews[session.id] ?? emptySessionView();
        const next = { ...state, openSessionIds: state.openSessionIds.includes(session.id) ? state.openSessionIds : [...state.openSessionIds, session.id], sessionViews: { ...state.sessionViews, [session.id]: view } };
        set({ ...next, ...projectActive(next, session.id), notice: state.sessionNotices[session.id] ?? null, actionError: state.sessionActionErrors[session.id] ?? null, actionCode: state.sessionActionCodes[session.id] ?? null, pendingDestination: state.sessionPendingDestinations[session.id] ?? null });
        if (!view.loaded && !view.messagesLoading) await get().loadLatestMessages();
      },
      closeSession(sessionId) {
        const state = get(); const index = state.openSessionIds.indexOf(sessionId); if (index < 0) return;
        const openSessionIds = state.openSessionIds.filter(id => id !== sessionId);
        const active = state.activeSessionId === sessionId ? (openSessionIds[index] ?? openSessionIds[index - 1] ?? null) : state.activeSessionId;
        set(current => { const next = { ...current, openSessionIds }; return { ...next, ...projectActive(next, active), notice: active ? next.sessionNotices[active] ?? null : null, actionError: active ? next.sessionActionErrors[active] ?? null : null, actionCode: active ? next.sessionActionCodes[active] ?? null : null, pendingDestination: active ? next.sessionPendingDestinations[active] ?? null : null }; });
      },
      async loadLatestMessages() {
        const session = get().activeSession; if (!session) return;
        const epoch = accountEpoch;
        const token = (messageTokens.get(session.id) ?? 0) + 1; messageTokens.set(session.id, token);
        set(state => ({ sessionViews: { ...state.sessionViews, [session.id]: { ...(state.sessionViews[session.id] ?? emptySessionView()), messagesLoading: true, messagesError: null } }, messagesLoading: true, messagesError: null }));
        try {
          const page = await call<ChatMessagePage>('list_chat_messages', { input: { session_id: session.id, limit: MESSAGE_PAGE } }); if (epoch !== accountEpoch || messageTokens.get(session.id) !== token) return;
          set(state => { const view = { ...(state.sessionViews[session.id] ?? emptySessionView()), messages: page.items, olderCursor: page.next_before_seq, newerTrimmed: false, messagesLoading: false, messagesError: null, loaded: true }; const next = { ...state, sessionViews: { ...state.sessionViews, [session.id]: view } }; return { ...next, ...projectActive(next, next.activeSessionId) }; });
        } catch (error) { if (epoch === accountEpoch && messageTokens.get(session.id) === token) { const text = chatErrorText(error); set(state => ({ sessionViews: { ...state.sessionViews, [session.id]: { ...(state.sessionViews[session.id] ?? emptySessionView()), messagesLoading: false, messagesError: text, loaded: true } }, messagesLoading: false, messagesError: text })); if (error instanceof ChatCommandError && error.code === 1006001) markStale(session.id); } }
      },
      async loadOlderMessages() {
        const session = get().activeSession; if (!session) return; const current = get().sessionViews[session.id] ?? emptySessionView(); if (current.olderCursor === null || current.messagesLoading) return;
        const epoch = accountEpoch;
        const token = (messageTokens.get(session.id) ?? 0) + 1; messageTokens.set(session.id, token); set({ messagesLoading: true, messagesError: null });
        try {
          const page = await call<ChatMessagePage>('list_chat_messages', { input: { session_id: session.id, before_seq: current.olderCursor, limit: MESSAGE_PAGE } }); if (epoch !== accountEpoch || messageTokens.get(session.id) !== token) return;
          set(state => { const view = state.sessionViews[session.id] ?? emptySessionView(); let messages = mergeById(page.items, view.messages, item => item.id); let newerTrimmed = view.newerTrimmed; if (messages.length > MESSAGE_CAP) { messages = messages.slice(0, MESSAGE_CAP); newerTrimmed = true; } const next = { ...state, sessionViews: { ...state.sessionViews, [session.id]: { ...view, messages, olderCursor: page.next_before_seq, newerTrimmed, messagesLoading: false } } }; return { ...next, ...projectActive(next, next.activeSessionId) }; });
        } catch (error) { if (epoch === accountEpoch && messageTokens.get(session.id) === token) set({ messagesLoading: false, messagesError: chatErrorText(error) }); }
      },
      setDraft(sessionId, draft) { set(state => ({ sessionDrafts: { ...state.sessionDrafts, [sessionId]: draft } })); },
      setSessionTargetDraft(sessionId, target) { set(state => ({ sessionTargetDrafts: { ...state.sessionTargetDrafts, [sessionId]: target } })); },
      setSessionTools(sessionId, tools) { set(state => ({ sessionTools: { ...state.sessionTools, [sessionId]: tools } })); },
      setSessionAssets(sessionId, assets) { set(state => ({ sessionAssets: { ...state.sessionAssets, [sessionId]: assets } })); },
      setSessionScrollTop(sessionId, top) { set(state => ({ sessionScrollTop: { ...state.sessionScrollTop, [sessionId]: top } })); },
      async renameSession(sessionId, title) { const token = accountEpoch; const trimmed = title.trim(); if (charCount(trimmed) < 1 || charCount(trimmed) > TITLE_MAX_CHARS) return chatErrorText(new ChatCommandError(1006007)); try { const value = await call<ChatSession>('rename_chat_session', { input: { session_id: sessionId, title: trimmed } }); if (token !== accountEpoch) return null; replaceSession(value); return null; } catch (error) { if (token === accountEpoch && error instanceof ChatCommandError && error.code === 1006001) markStale(sessionId); return token === accountEpoch ? chatErrorText(error) : null; } },
      async deleteSession(sessionId) { const token = accountEpoch; try { await call<unknown>('delete_chat_session', { input: { session_id: sessionId } }); } catch (error) { if (token !== accountEpoch) return null; if (!(error instanceof ChatCommandError && error.code === 1006001)) return chatErrorText(error); } if (token !== accountEpoch) return null; runs.delete(sessionId); messageTokens.delete(sessionId); set(state => { const openSessionIds = state.openSessionIds.filter(id => id !== sessionId); const active = state.activeSessionId === sessionId ? (openSessionIds[0] ?? null) : state.activeSessionId; const next = { ...state, sessions: state.sessions.filter(item => item.id !== sessionId), searchResults: state.searchResults.filter(hit => hit.session.id !== sessionId), openSessionIds }; return { ...next, ...projectActive(next, active) }; }); return null; },
      async updateTarget(sessionId, target) { const token = accountEpoch; try { const value = await call<ChatSession>('update_chat_session_target', { input: { session_id: sessionId, ...target } }); if (token !== accountEpoch) return null; replaceSession(value); return null; } catch (error) { if (token === accountEpoch && error instanceof ChatCommandError && error.code === 1006001) markStale(sessionId); return token === accountEpoch ? chatErrorText(error) : null; } },
      async send(content, tools = [], acceptDestination, assetIds) {
        const { activeSession, activeSessionId, activeStale, generation } = get();
        if (!activeSession || !activeSessionId || activeStale || generation || runs.size > 0) return 'rejected';
        const ids = assetIds ?? (get().sessionAssets[activeSessionId] ?? []).map(asset => asset.id);
        if (!content.trim() && !ids.length) { set({ actionError: t('chat.emptyMessage') }); return 'rejected'; }
        if (utf8Bytes(content) > MAX_MESSAGE_BYTES) { set({ actionError: t('chat.tooLong') }); return 'rejected'; }
        if (tools.length > MAX_TOOLS) { set({ actionError: t('chat.toolLimit') }); return 'rejected'; }
        if (get().newerTrimmed) await get().loadLatestMessages();
        if (get().activeSessionId !== activeSessionId || get().generation || runs.size > 0) return 'rejected';
        const requestId = deps.newRequestId();
        const input: Record<string, unknown> = { session_id: activeSessionId, content, client_request_id: requestId, ...(ids.length ? { asset_ids: ids } : {}) };
        if (acceptDestination) input.accept_destination = acceptDestination;
        if (tools.length) input.tools = tools.map(tool => ({ server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash }));
        return start('send', activeSessionId, 'send_chat_message', input, requestId, content, tools.length > 0, null, () => ({ kind: 'send', sessionId: activeSessionId, content, tools, assetIds: ids, fingerprint: '', host: '' }));
      },
      async regenerate(messageId, acceptDestination) { const { activeSession, activeSessionId, messages, newerTrimmed } = get(); if (!activeSession || !activeSessionId || get().activeStale || get().generation || runs.size > 0) return 'rejected'; const last = messages[messages.length - 1]; if (newerTrimmed || !last || last.id !== messageId || last.role !== 'assistant') { set({ actionError: chatErrorText(new ChatCommandError(1006014)), actionCode: 1006014 }); return 'rejected'; } const requestId = deps.newRequestId(); const input: Record<string, unknown> = { message_id: messageId, client_request_id: requestId }; if (acceptDestination) input.accept_destination = acceptDestination; return start('regenerate', activeSessionId, 'regenerate_chat_message', input, requestId, '', false, messageId, () => ({ kind: 'regenerate', sessionId: activeSessionId, messageId, fingerprint: '', host: '' })); },
      async stop() { const id = get().activeSessionId; if (id) await get().stopSession(id); },
      async stopSession(sessionId) { const r = runs.get(sessionId); if (!r) return; r.stopRequested = true; patchGeneration(r, { stopping: true }); if (r.acked) await cancelRun(r); },
      async cancelAll() { await Promise.all([...runs.keys()].map(sessionId => get().stopSession(sessionId))); },
      cancelOnLeave() { /* Route and inner-tab switches are not cancellation boundaries. */ },
      async decideTool(type) { const id = get().activeSessionId; const r = id ? runs.get(id) : undefined; const generation = id ? get().generations[id] : null; if (!r || !generation?.proposal || generation.proposalBusy) return; patchGeneration(r, { proposalBusy: true }); try { await call<unknown>('advance_chat_tool_turn', { input: { task_id: r.taskId, action: { type, proposal_id: generation.proposal.id } } }); patchGeneration(r, { phase: 'tool_running', proposal: null, proposalBusy: false }); } catch (error) { if (runs.get(r.sessionId) !== r) return; const stale = error instanceof ChatCommandError && error.code === 1006017; patchGeneration(r, stale ? { proposal: null, proposalBusy: false } : { proposalBusy: false }); const text = chatErrorText(error); set(state => ({ actionError: text, actionCode: error instanceof ChatCommandError ? error.code : null, sessionActionErrors: { ...state.sessionActionErrors, [id!]: text }, sessionActionCodes: { ...state.sessionActionCodes, [id!]: error instanceof ChatCommandError ? error.code : null } })); } },
      confirmDestination() { const pending = get().pendingDestination; if (!pending) return; set(state => ({ pendingDestination: null, sessionPendingDestinations: { ...state.sessionPendingDestinations, [pending.sessionId]: null } })); if (pending.kind === 'send') void get().send(pending.content, pending.tools, pending.fingerprint, pending.assetIds); else void get().regenerate(pending.messageId, pending.fingerprint); },
      dismissDestination(token) { const pending = get().pendingDestination; if (token && token !== pending) return; set(state => ({ pendingDestination: null, ...(pending ? { sessionPendingDestinations: { ...state.sessionPendingDestinations, [pending.sessionId]: null } } : {}) })); },
      dismissNotice() { const id = get().activeSessionId; if (id) set(state => ({ notice: null, sessionNotices: { ...state.sessionNotices, [id]: null } })); },
      reset() { ++accountEpoch; clearTimeout(searchTimer); searchTimer = undefined; runs.clear(); messageTokens.clear(); ++sessionsToken; ++searchToken; set(initialData()); },
    };
  });
}

export const useChatStore = createChatStore();

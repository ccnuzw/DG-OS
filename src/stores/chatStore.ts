import { t } from '../i18n';
import { create } from 'zustand';
import { Channel, invoke as tauriInvoke } from '@tauri-apps/api/core';
import { extractErrorCode, lookupError } from '@dgos/ui';
import type { GenerationParameters } from '../components/chat/GenerationControls';
import { defaultAiFoundation, type AiFoundationOptions, type AiResource, type AiResourceImport, type AiContextRef } from '../types/ai';

export interface ChatFoundationSettings { context_refs: AiContextRef[]; output_schema: string | null }
function chatFoundation(settings?: ChatFoundationSettings): AiFoundationOptions {
  return { ...defaultAiFoundation(), context_refs: settings?.context_refs ?? [], output: settings?.output_schema == null ? { kind: 'text' } : { kind: 'json_schema', schema: JSON.parse(settings.output_schema) } };
}

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
  dispatched?: boolean;
  audit_warning?: string | null;
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
export interface ChatSkillPin { skill_id: string; version: string; content_hash: string }
export interface ChatSkillEntry { pin: ChatSkillPin; name: string; description: string; category: string; tags: string[] }
export interface ChatSkillRecommendation { entry: ChatSkillEntry; score: number; reasons: string[] }
export interface ChatSkillAccess { skill_mode: 'manual' | 'on_demand'; authorized_skill_refs: ChatSkillPin[] }
function skillAccessWire(access?: Partial<ChatSkillAccess>): Partial<ChatSkillAccess> {
  return access?.skill_mode ? { skill_mode: access.skill_mode, authorized_skill_refs: access.skill_mode === 'on_demand' ? access.authorized_skill_refs ?? [] : [] } : {};
}
export interface SendAck {
  task_id: string; user_message_id: string; assistant_message_id: string;
  dropped_turns: number; replayed: boolean;
}
export interface ChatToolProposal {
  id: string; server_id: string; server_name: string; tool_name: string;
  schema_hash: string; arguments: unknown; expires_at: number; // Unix seconds
}
export interface ChatSummary {
  id: string; source_first_seq: number; source_last_seq: number; source_digest: string;
  content: string; provider_id: string; model_id: string; created_at: number;
}
export interface ChatContextBudget {
  model_id: string; model_window_tokens: number | null; host_text_limit_bytes: number;
  reserved_output_tokens: number; estimated_input_tokens: number; text_bytes: number;
  image_reserve_tokens: number; skill_bytes: number; tool_schema_bytes: number; history_bytes: number;
  dropped_turns: number; summary: ChatSummary | null; can_send: boolean; rejection: string | null;
}
export interface ChatRunStep {
  id: string; ordinal: number; kind: string; phase: string; status: string; dispatched: boolean;
  result_unknown: boolean; receipt: Record<string, unknown> | null; created_at: number; updated_at: number;
}
export interface ChatRun {
  task_id: string; session_id: string; assistant_message_id: string; source_run_id: string | null;
  status: string; partial_content: string; dispatched: boolean; result_unknown: boolean;
  audit_warning: string | null; total_tokens: number | null; steps: ChatRunStep[]; created_at: number; updated_at: number;
}
export interface ChatTerminalEvidence { dispatched: boolean; result_unknown: boolean; audit_warning: string | null }
export type ChatStreamEvent =
  | { kind: 'started'; task_id: string; assistant_message_id: string }
  | { kind: 'delta'; assistant_message_id: string; text: string }
  | { kind: 'reasoning_delta'; assistant_message_id: string; text: string }
  | { kind: 'phase'; task_id: string; assistant_message_id: string; phase: string }
  | { kind: 'activity'; task_id: string; assistant_message_id: string; step: ChatRunStep }
  | ({ kind: 'summary_done'; task_id: string; assistant_message_id: string; summary: ChatSummary } & ChatTerminalEvidence)
  | { kind: 'tool_proposal'; assistant_message_id: string; proposal: ChatToolProposal }
  | ({ kind: 'done'; message: ChatMessage } & Partial<ChatTerminalEvidence>)
  | { kind: 'error'; assistant_message_id: string; message: ChatMessage | null;
      error_code: string; provider_code: number | null;
      result_unknown: boolean; regeneration_discarded: boolean; dispatched?: boolean; audit_warning?: string | null };

interface CommandResponse<T> {
  success: boolean; data?: T; error?: { code?: number; message?: string; detail?: unknown };
}

export const SESSION_PAGE = 30;
export const SESSION_CAP = 300;
export const MESSAGE_PAGE = 50;
export const MESSAGE_CAP = 200;
export const MAX_MESSAGE_BYTES = 16384;
export const MAX_INTENT_BYTES = 8192;
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
  kind: 'send' | 'regenerate' | 'summary' | 'continue';
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
  hostPhase?: string;
  activities?: ChatRunStep[];
}
export interface ChatNotice { tone: 'error' | 'warning' | 'info'; text: string }
export type PendingDestination =
  | { kind: 'send'; sessionId: string; content: string; tools: ChatToolSelection[]; assetIds?: string[]; fingerprint: string; host: string }
  | { kind: 'regenerate'; sessionId: string; messageId: string; fingerprint: string; host: string }
  | { kind: 'summary'; sessionId: string; fingerprint: string; host: string }
  | { kind: 'continue'; sessionId: string; sourceTaskId: string; intent: string; tools: ChatToolSelection[]; fingerprint: string; host: string };
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
  sessionRememberTools: Record<string, boolean>;
  sessionUnknown: Record<string, boolean>;
  terminalEvidence: Record<string, ChatTerminalEvidence>;
  sessionSummaries: Record<string, ChatSummary>;
  sessionSkillAccess: Record<string, ChatSkillAccess>;
  sessionFoundation: Record<string, ChatFoundationSettings>;
  sessionResources: Record<string, AiResource[]>;
  runIntentDrafts: Record<string, string>;
  sessionAssets: Record<string, ChatAsset[]>;
  sessionScrollTop: Record<string, number>;
  generations: Record<string, Generation>;
  preparingSessionId: string | null;

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
  setRememberTools: (sessionId: string, remember: boolean) => void;
  setSkillAccess: (sessionId: string, access: ChatSkillAccess) => void;
  setFoundation: (sessionId: string, settings: ChatFoundationSettings) => void;
  importResource: (sessionId: string, input: AiResourceImport) => Promise<AiResource | null>;
  deleteResource: (sessionId: string, resourceId: string) => Promise<void>;
  skillCatalog: (query: string) => Promise<{ items: ChatSkillEntry[]; recommendations: ChatSkillRecommendation[] } | null>;
  inspectContext: (sessionId: string, content: string, tools?: ChatToolSelection[], assetIds?: string[], foundation?: AiFoundationOptions, skillAccess?: Partial<ChatSkillAccess>) => Promise<ChatContextBudget | null>;
  summarize: (acceptDestination?: string) => Promise<SendOutcome>;
  listRuns: (sessionId: string) => Promise<ChatRun[] | null>;
  inspectRun: (taskId: string) => Promise<ChatRun | null>;
  resolveRunStep: (taskId: string, stepId: string, outcome: 'completed' | 'not_executed', note: string) => Promise<ChatRun | null>;
  continueRun: (sourceTaskId: string, intent: string, tools?: ChatToolSelection[], acceptDestination?: string) => Promise<SendOutcome>;
  setRunIntent: (taskId: string, intent: string) => void;
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
  | 'decideTool' | 'confirmDestination' | 'dismissDestination' | 'dismissNotice' | 'reset'
  | 'setRememberTools' | 'inspectContext' | 'summarize' | 'listRuns' | 'inspectRun' | 'continueRun' | 'setSkillAccess' | 'skillCatalog' | 'setRunIntent' | 'resolveRunStep' | 'setFoundation' | 'importResource' | 'deleteResource'>;

const initialData = (): Data => ({
  sessions: [], sessionsCursor: null, sessionsCapped: false, sessionsLoading: false, sessionsLoaded: false, sessionsError: null,
  searchQuery: '', searchResults: [], searchCursor: null, searchCapped: false, searchLoading: false, searchError: null,
  sessionNotices: {}, sessionActionErrors: {}, sessionActionCodes: {}, sessionPendingDestinations: {},
  openSessionIds: [], activeSessionId: null, sessionViews: {}, sessionDrafts: {}, sessionTargetDrafts: {}, sessionTools: {}, sessionAssets: {}, sessionScrollTop: {}, generations: {},
  sessionRememberTools: {}, sessionUnknown: {}, terminalEvidence: {}, sessionSummaries: {},
  sessionSkillAccess: {}, sessionFoundation: {}, sessionResources: {},
  runIntentDrafts: {},
  preparingSessionId: null,
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
  kind: Generation['kind'];
  content: string;
  assets: ChatAsset[];
  taskId: string;
  acked: boolean;
  assistantId: string | null;
  buffer: ChatStreamEvent[];
  stopRequested: boolean;
  cancellationError?: string;
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
  let preparation: { sessionId: string; epoch: number } | null = null;
  // A missing page item cannot resolve an uncertain dispatch. Only the exact run's evidence can.
  const uncertainRuns = new Map<string, Set<string>>();
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
    function reservePreparation(sessionId: string) {
      if (preparation || runs.size) return null;
      const token = { sessionId, epoch: accountEpoch };
      preparation = token;
      set({ preparingSessionId: sessionId });
      return token;
    }
    function releasePreparation(token: NonNullable<typeof preparation>) {
      if (preparation !== token) return;
      preparation = null;
      set({ preparingSessionId: null });
    }
    function runUncertainty(sessionId: string, taskId: string, uncertain: boolean): boolean {
      const ids = uncertainRuns.get(sessionId) ?? new Set<string>();
      if (uncertain) ids.add(taskId); else ids.delete(taskId);
      if (ids.size) uncertainRuns.set(sessionId, ids); else uncertainRuns.delete(sessionId);
      return ids.size > 0;
    }
    function reconcileRuns(sessionId: string, items: ChatRun[]) {
      for (const run of items) {
        if (run.session_id !== sessionId) continue;
        const unsafe = run.result_unknown || !!run.audit_warning || run.steps.some(step => step.result_unknown || (step.kind === 'tool' && (step.status === 'dispatched' || (step.dispatched && !step.receipt))));
        const terminal = ['completed', 'succeeded', 'failed', 'cancelled', 'interrupted'].includes(run.status);
        if (unsafe || terminal) runUncertainty(sessionId, run.task_id, unsafe);
      }
      set(state => ({ sessionUnknown: { ...state.sessionUnknown, [sessionId]: (uncertainRuns.get(sessionId)?.size ?? 0) > 0 } }));
    }
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
        if (runs.get(r.sessionId) !== r) return;
        set(state => {
          if (!r.cancellationError || state.sessionActionErrors[r.sessionId] !== r.cancellationError) return {};
          return { sessionActionErrors: { ...state.sessionActionErrors, [r.sessionId]: null }, ...(state.activeSessionId === r.sessionId ? { actionError: null } : {}) };
        });
      } catch {
        if (runs.get(r.sessionId) !== r) return;
        r.stopRequested = false;
        patchGeneration(r, { stopping: false });
        set(state => {
          const text = t('chat.stopUnconfirmed');
          r.cancellationError = text;
          return { sessionActionErrors: { ...state.sessionActionErrors, [r.sessionId]: text }, ...(state.activeSessionId === r.sessionId ? { actionError: text } : {}) };
        });
      }
    }

    function finalize(r: Run, snapshot: ChatMessage | null, error: Extract<ChatStreamEvent, { kind: 'error' }> | null, evidence?: Partial<ChatTerminalEvidence>) {
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
      if (evidence?.audit_warning) notice = { tone: 'warning', text: `${notice?.text ?? ''} ${t('chat.auditWarning')}`.trim() };
      let reload = false;
      set(state => {
        if (r.cancellationError && state.sessionActionErrors[r.sessionId] === r.cancellationError) {
          state = { ...state, sessionActionErrors: { ...state.sessionActionErrors, [r.sessionId]: null } };
        }
        const view = state.sessionViews[r.sessionId] ?? emptySessionView();
        let messages = view.messages;
        if (snapshot) {
          const keepOld = error?.regeneration_discarded === true;
          if (replaced && !keepOld && snapshot.id !== replaced) messages = messages.filter(item => item.id !== replaced);
          messages = upsertMessage(messages, snapshot);
        } else if (r.kind !== 'summary') reload = true;
        if (messages.length > MESSAGE_CAP) reload = true;
        const terminal = { dispatched: evidence?.dispatched ?? error?.dispatched ?? snapshot?.dispatched ?? false, result_unknown: evidence?.result_unknown ?? error?.result_unknown ?? snapshot?.result_unknown ?? false, audit_warning: evidence?.audit_warning ?? error?.audit_warning ?? snapshot?.audit_warning ?? null };
        const next = { ...state, sessionViews: { ...state.sessionViews, [r.sessionId]: { ...view, messages } }, generations: Object.fromEntries(Object.entries(state.generations).filter(([id]) => id !== r.sessionId)), sessionNotices: { ...state.sessionNotices, [r.sessionId]: notice }, sessionUnknown: { ...state.sessionUnknown, [r.sessionId]: runUncertainty(r.sessionId, r.taskId, terminal.result_unknown || !!terminal.audit_warning) }, terminalEvidence: { ...state.terminalEvidence, [r.assistantId ?? r.id]: terminal } };
        return { ...next, ...projectActive(next, state.activeSessionId) };
      });
      if (reload && get().activeSessionId === r.sessionId) void get().loadLatestMessages();
      void refreshSessions();
    }

    function applyEvent(r: Run, event: ChatStreamEvent) {
      if (runs.get(r.sessionId) !== r || !event || typeof event !== 'object') return;
      const id = eventAssistantId(event);
      if (id !== null && r.assistantId !== null && id !== r.assistantId) return;
      if ('task_id' in event && event.task_id !== r.taskId) return;
      switch (event.kind) {
        case 'started': return;
        case 'delta': return patchGeneration(r, g => ({ text: g.text + event.text }));
        case 'reasoning_delta': return patchGeneration(r, g => ({ reasoning: g.reasoning + event.text }));
        case 'phase': return patchGeneration(r, { hostPhase: event.phase });
        case 'activity': return patchGeneration(r, g => ({ activities: [...(g.activities ?? []).filter(step => step.id !== event.step.id), event.step].sort((a, b) => a.ordinal - b.ordinal) }));
        case 'summary_done': set(state => ({ sessionSummaries: { ...state.sessionSummaries, [r.sessionId]: event.summary } })); return finalize(r, null, null, event);
        case 'tool_proposal': return patchGeneration(r, { phase: 'awaiting_tool', proposal: event.proposal, proposalBusy: false });
        case 'done': return finalize(r, event.message, null, event);
        case 'error': return finalize(r, event.message, event, event);
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
        if ((r.kind === 'send' || r.kind === 'continue') && !messages.some(m => m.id === ack.user_message_id)) {
          const now = Math.floor(Date.now() / 1000);
          messages = [...messages, { id: ack.user_message_id, session_id: r.sessionId, seq: (messages[messages.length - 1]?.seq ?? 0) + 1, role: 'user', content: r.content, assets: r.assets, status: 'completed', task_id: ack.task_id, finish_reason: null, error_code: null, total_tokens: null, result_unknown: false, created_at: now, updated_at: now }];
        }
        const current = state.generations[r.sessionId];
        const generation = current?.runId === r.id ? { ...current, assistantMessageId: ack.assistant_message_id, phase: current.phase === 'sending' ? 'running' as const : current.phase } : current;
        const sessionDrafts = r.kind === 'send' && state.sessionDrafts[r.sessionId] === r.content ? { ...state.sessionDrafts, [r.sessionId]: '' } : state.sessionDrafts;
        const sessionTools = r.kind === 'send' && !state.sessionRememberTools[r.sessionId] ? { ...state.sessionTools, [r.sessionId]: [] } : state.sessionTools;
        const next = { ...state, sessionDrafts, sessionTools, sessionViews: { ...state.sessionViews, [r.sessionId]: { ...view, messages, droppedTurns: ack.dropped_turns > 0 ? { ...view.droppedTurns, [ack.assistant_message_id]: ack.dropped_turns } : view.droppedTurns } }, generations: generation ? { ...state.generations, [r.sessionId]: generation } : state.generations, lastSent: r.kind === 'send' ? { seq: state.lastSent.seq + 1, sessionId: r.sessionId } : state.lastSent };
        return { ...next, ...projectActive(next, state.activeSessionId) };
      });
      if (r.kind === 'send') set(state => ({ sessionAssets: { ...state.sessionAssets, [r.sessionId]: (state.sessionAssets[r.sessionId] ?? []).filter(asset => !r.assets.some(sent => sent.id === asset.id)) } }));
      for (const event of r.buffer.splice(0)) applyEvent(r, event);
      if (r.stopRequested && runs.get(r.sessionId) === r) void cancelRun(r);
    }

    async function start(kind: Generation['kind'], sessionId: string, command: string, input: Record<string, unknown>, requestId: string, content: string, toolsMode: boolean, replacesMessageId: string | null, pending: () => PendingDestination): Promise<SendOutcome> {
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
        const text = code === null ? t('chat.requestUnconfirmed') : chatErrorText(error);
        set(state => ({ sessionActionErrors: { ...state.sessionActionErrors, [sessionId]: text }, sessionActionCodes: { ...state.sessionActionCodes, [sessionId]: code }, ...(state.activeSessionId === sessionId ? { actionError: text, actionCode: code } : {}) }));
        if (code === null) set(state => ({ sessionUnknown: { ...state.sessionUnknown, [sessionId]: runUncertainty(sessionId, r.taskId, true) } }));
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
        if (preparation?.sessionId === sessionId) releasePreparation(preparation);
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
        } catch (error) { if (epoch === accountEpoch && messageTokens.get(session.id) === token) { const text = chatErrorText(error); set(state => { const next = { ...state, sessionViews: { ...state.sessionViews, [session.id]: { ...(state.sessionViews[session.id] ?? emptySessionView()), messagesLoading: false, messagesError: text, loaded: true } } }; return { ...next, ...projectActive(next, next.activeSessionId) }; }); if (error instanceof ChatCommandError && error.code === 1006001) markStale(session.id); } }
      },
      async loadOlderMessages() {
        const session = get().activeSession; if (!session) return; const current = get().sessionViews[session.id] ?? emptySessionView(); if (current.olderCursor === null || current.messagesLoading) return;
        const epoch = accountEpoch;
        const token = (messageTokens.get(session.id) ?? 0) + 1; messageTokens.set(session.id, token); set({ messagesLoading: true, messagesError: null });
        try {
          const page = await call<ChatMessagePage>('list_chat_messages', { input: { session_id: session.id, before_seq: current.olderCursor, limit: MESSAGE_PAGE } }); if (epoch !== accountEpoch || messageTokens.get(session.id) !== token) return;
          set(state => { const view = state.sessionViews[session.id] ?? emptySessionView(); let messages = mergeById(page.items, view.messages, item => item.id); let newerTrimmed = view.newerTrimmed; if (messages.length > MESSAGE_CAP) { messages = messages.slice(0, MESSAGE_CAP); newerTrimmed = true; } const next = { ...state, sessionViews: { ...state.sessionViews, [session.id]: { ...view, messages, olderCursor: page.next_before_seq, newerTrimmed, messagesLoading: false } } }; return { ...next, ...projectActive(next, next.activeSessionId) }; });
        } catch (error) { if (epoch === accountEpoch && messageTokens.get(session.id) === token) set(state => { const next = { ...state, sessionViews: { ...state.sessionViews, [session.id]: { ...(state.sessionViews[session.id] ?? emptySessionView()), messagesLoading: false, messagesError: chatErrorText(error) } } }; return { ...next, ...projectActive(next, next.activeSessionId) }; }); }
      },
      setDraft(sessionId, draft) { set(state => ({ sessionDrafts: { ...state.sessionDrafts, [sessionId]: draft } })); },
      setSessionTargetDraft(sessionId, target) { set(state => ({ sessionTargetDrafts: { ...state.sessionTargetDrafts, [sessionId]: target } })); },
      setSessionTools(sessionId, tools) { set(state => ({ sessionTools: { ...state.sessionTools, [sessionId]: tools } })); },
      setRememberTools(sessionId, remember) { set(state => ({ sessionRememberTools: { ...state.sessionRememberTools, [sessionId]: remember } })); },
      setSkillAccess(sessionId, access) { set(state => ({ sessionSkillAccess: { ...state.sessionSkillAccess, [sessionId]: access } })); },
      setFoundation(sessionId, settings) { set(state => ({ sessionFoundation: { ...state.sessionFoundation, [sessionId]: settings } })); },
      async importResource(sessionId, input) {
        const epoch = accountEpoch;
        const resource = await call<AiResource>('chat_import_ai_resource', { sessionId, input });
        if (epoch !== accountEpoch) return null;
        set(state => ({ sessionResources: { ...state.sessionResources, [sessionId]: [...(state.sessionResources[sessionId] ?? []), resource] } }));
        return resource;
      },
      async deleteResource(sessionId, resourceId) {
        const epoch = accountEpoch;
        await call<boolean>('chat_delete_ai_resource', { sessionId, resourceId });
        if (epoch !== accountEpoch) return;
        set(state => ({ sessionResources: { ...state.sessionResources, [sessionId]: (state.sessionResources[sessionId] ?? []).filter(resource => resource.id !== resourceId) }, sessionFoundation: { ...state.sessionFoundation, [sessionId]: { output_schema: state.sessionFoundation[sessionId]?.output_schema ?? null, context_refs: (state.sessionFoundation[sessionId]?.context_refs ?? []).filter(ref => ref.resource_id !== resourceId) } } }));
      },
      setRunIntent(taskId, intent) { set(state => ({ runIntentDrafts: { ...state.runIntentDrafts, [taskId]: intent } })); },
      async skillCatalog(query) {
        const epoch = accountEpoch;
        const catalog = await call<{ items: ChatSkillEntry[]; recommendations: ChatSkillRecommendation[] }>('chat_skill_catalog', { input: { query } });
        return epoch === accountEpoch ? catalog : null;
      },
      async inspectContext(sessionId, content, tools = [], assetIds, foundation, skillAccess) {
        const epoch = accountEpoch;
        const budget = await call<ChatContextBudget>('inspect_chat_context', { input: { session_id: sessionId, content, asset_ids: assetIds ?? (get().sessionAssets[sessionId] ?? []).map(asset => asset.id), tools, ...skillAccessWire(skillAccess ?? get().sessionSkillAccess[sessionId]), foundation: foundation ?? chatFoundation(get().sessionFoundation[sessionId]) } });
        return epoch === accountEpoch ? budget : null;
      },
      async summarize(acceptDestination) {
        const sessionId = get().activeSessionId;
        if (!sessionId || preparation || get().activeStale || runs.size || get().sessionUnknown[sessionId]) return 'rejected';
        const requestId = deps.newRequestId();
        return start('summary', sessionId, 'summarize_chat_context', { session_id: sessionId, client_request_id: requestId, confirmed: true, ...(acceptDestination ? { accept_destination: acceptDestination } : {}) }, requestId, '', false, null, () => ({ kind: 'summary', sessionId, fingerprint: '', host: '' }));
      },
      async listRuns(sessionId) {
        const epoch = accountEpoch;
        const page = await call<{ items: ChatRun[] }>('list_chat_runs', { input: { session_id: sessionId } });
        if (epoch !== accountEpoch) return null;
        reconcileRuns(sessionId, page.items);
        return page.items;
      },
      async inspectRun(taskId) {
        const epoch = accountEpoch;
        const run = await call<ChatRun>('get_chat_run', { input: { task_id: taskId } });
        if (epoch !== accountEpoch || run.task_id !== taskId) return null;
        reconcileRuns(run.session_id, [run]);
        return run;
      },
      async resolveRunStep(taskId, stepId, outcome, note) {
        const epoch = accountEpoch;
        if (!note.trim() || utf8Bytes(note) > 2048 || runs.size) return null;
        const run = await call<ChatRun>('resolve_chat_run_step', { input: { task_id: taskId, step_id: stepId, confirmed: true, outcome, note } });
        if (epoch !== accountEpoch) return null;
        const page = await call<{ items: ChatRun[] }>('list_chat_runs', { input: { session_id: run.session_id } });
        if (epoch !== accountEpoch) return null;
        reconcileRuns(run.session_id, [run, ...page.items]);
        if (get().activeSessionId === run.session_id) await get().loadLatestMessages();
        return run;
      },
      async continueRun(sourceTaskId, intent, tools = [], acceptDestination) {
        const sessionId = get().activeSessionId;
        const epoch = accountEpoch;
        if (!sessionId || !intent.trim() || utf8Bytes(intent) > MAX_INTENT_BYTES || get().activeStale) return 'rejected';
        const token = reservePreparation(sessionId);
        if (!token) return 'rejected';
        try {
          const source = await get().inspectRun(sourceTaskId);
          if (!source || epoch !== accountEpoch || preparation !== token || get().activeSessionId !== sessionId || source.session_id !== sessionId || runs.size) return 'rejected';
          if (source.audit_warning || source.steps.some(step => step.kind === 'tool' && (step.result_unknown || step.status === 'dispatched' || (step.dispatched && !step.receipt))) || ['running', 'generating', 'pending'].includes(source.status)) { set({ actionError: t('chat.continueBlocked') }); return 'rejected'; }
          const requestId = deps.newRequestId();
          return await start('continue', sessionId, 'continue_chat_run', { source_task_id: sourceTaskId, client_request_id: requestId, intent, tools, ...skillAccessWire(get().sessionSkillAccess[sessionId]), foundation: chatFoundation(get().sessionFoundation[sessionId]), ...(acceptDestination ? { accept_destination: acceptDestination } : {}) }, requestId, intent, tools.length > 0, null, () => ({ kind: 'continue', sessionId, sourceTaskId, intent, tools, fingerprint: '', host: '' }));
        } catch (error) { if (epoch === accountEpoch && get().activeSessionId === sessionId) set({ actionError: chatErrorText(error) }); return 'rejected'; }
        finally { releasePreparation(token); }
      },
      setSessionAssets(sessionId, assets) { set(state => ({ sessionAssets: { ...state.sessionAssets, [sessionId]: assets } })); },
      setSessionScrollTop(sessionId, top) { set(state => ({ sessionScrollTop: { ...state.sessionScrollTop, [sessionId]: top } })); },
      async renameSession(sessionId, title) { const token = accountEpoch; const trimmed = title.trim(); if (charCount(trimmed) < 1 || charCount(trimmed) > TITLE_MAX_CHARS) return chatErrorText(new ChatCommandError(1006007)); try { const value = await call<ChatSession>('rename_chat_session', { input: { session_id: sessionId, title: trimmed } }); if (token !== accountEpoch) return null; replaceSession(value); return null; } catch (error) { if (token === accountEpoch && error instanceof ChatCommandError && error.code === 1006001) markStale(sessionId); return token === accountEpoch ? chatErrorText(error) : null; } },
      async deleteSession(sessionId) { const token = accountEpoch; try { await call<unknown>('delete_chat_session', { input: { session_id: sessionId } }); } catch (error) { if (token !== accountEpoch) return null; if (!(error instanceof ChatCommandError && error.code === 1006001)) return chatErrorText(error); } if (token !== accountEpoch) return null; runs.delete(sessionId); messageTokens.delete(sessionId); set(state => { const openSessionIds = state.openSessionIds.filter(id => id !== sessionId); const active = state.activeSessionId === sessionId ? (openSessionIds[0] ?? null) : state.activeSessionId; const next = { ...state, sessions: state.sessions.filter(item => item.id !== sessionId), searchResults: state.searchResults.filter(hit => hit.session.id !== sessionId), openSessionIds }; return { ...next, ...projectActive(next, active) }; }); return null; },
      async updateTarget(sessionId, target) { const token = accountEpoch; try { const value = await call<ChatSession>('update_chat_session_target', { input: { session_id: sessionId, ...target } }); if (token !== accountEpoch) return null; replaceSession(value); return null; } catch (error) { if (token === accountEpoch && error instanceof ChatCommandError && error.code === 1006001) markStale(sessionId); return token === accountEpoch ? chatErrorText(error) : null; } },
      async send(content, tools = [], acceptDestination, assetIds) {
        const { activeSession, activeSessionId, activeStale, generation } = get();
        if (!activeSession || !activeSessionId || activeStale || generation) return 'rejected';
        if (get().sessionUnknown[activeSessionId] || get().messages.some(message => message.result_unknown || !!message.audit_warning)) { set({ actionError: t('chat.continueBlocked') }); return 'rejected'; }
        const ids = assetIds ?? (get().sessionAssets[activeSessionId] ?? []).map(asset => asset.id);
        if (!content.trim() && !ids.length && !get().sessionFoundation[activeSessionId]?.context_refs.length) { set({ actionError: t('chat.emptyMessage') }); return 'rejected'; }
        if (utf8Bytes(content) > MAX_MESSAGE_BYTES) { set({ actionError: t('chat.tooLong') }); return 'rejected'; }
        if (tools.length > MAX_TOOLS) { set({ actionError: t('chat.toolLimit') }); return 'rejected'; }
        const token = reservePreparation(activeSessionId);
        if (!token) return 'rejected';
        const epoch = accountEpoch;
        try {
          if (get().newerTrimmed) await get().loadLatestMessages();
          if (epoch !== accountEpoch || preparation !== token || get().activeSessionId !== activeSessionId || get().generation || runs.size) return 'rejected';
          const foundation = chatFoundation(get().sessionFoundation[activeSessionId]);
          const skillAccess = skillAccessWire(get().sessionSkillAccess[activeSessionId]);
          const budget = await get().inspectContext(activeSessionId, content, tools, ids, foundation, skillAccess);
          if (!budget || epoch !== accountEpoch || preparation !== token || get().activeSessionId !== activeSessionId || runs.size) return 'rejected';
          if (!budget.can_send) { set({ actionError: t('chat.contextRejected'), actionCode: 1006008 }); return 'rejected'; }
          const requestId = deps.newRequestId();
          const input: Record<string, unknown> = { session_id: activeSessionId, content, client_request_id: requestId, ...(ids.length ? { asset_ids: ids } : {}), ...skillAccess, foundation };
          if (acceptDestination) input.accept_destination = acceptDestination;
          if (tools.length) input.tools = tools.map(tool => ({ server_id: tool.server_id, tool_name: tool.tool_name, schema_hash: tool.schema_hash }));
          return await start('send', activeSessionId, 'send_chat_message', input, requestId, content, tools.length > 0, null, () => ({ kind: 'send', sessionId: activeSessionId, content, tools, assetIds: ids, fingerprint: '', host: '' }));
        } catch (error) { if (epoch === accountEpoch && get().activeSessionId === activeSessionId) set({ actionError: chatErrorText(error) }); return 'rejected'; }
        finally { releasePreparation(token); }
      },
      async regenerate(messageId, acceptDestination) {
        const { activeSession, activeSessionId, messages, newerTrimmed } = get();
        if (!activeSession || !activeSessionId || preparation || get().activeStale || get().generation || runs.size > 0 || get().sessionUnknown[activeSessionId] || messages.some(message => message.result_unknown || !!message.audit_warning)) return 'rejected';
        const last = messages[messages.length - 1];
        if (newerTrimmed || !last || last.id !== messageId || last.role !== 'assistant') { set({ actionError: chatErrorText(new ChatCommandError(1006014)), actionCode: 1006014 }); return 'rejected'; }
        let foundation: AiFoundationOptions;
        try { foundation = chatFoundation(get().sessionFoundation[activeSessionId]); }
        catch { set({ actionError: t('chat.invalidOutputSchema') }); return 'rejected'; }
        const requestId = deps.newRequestId();
        const input: Record<string, unknown> = { message_id: messageId, client_request_id: requestId, ...skillAccessWire(get().sessionSkillAccess[activeSessionId]), foundation };
        if (acceptDestination) input.accept_destination = acceptDestination;
        return start('regenerate', activeSessionId, 'regenerate_chat_message', input, requestId, '', false, messageId, () => ({ kind: 'regenerate', sessionId: activeSessionId, messageId, fingerprint: '', host: '' }));
      },
      async stop() { const id = get().activeSessionId; if (id) await get().stopSession(id); },
      async stopSession(sessionId) { if (preparation?.sessionId === sessionId) releasePreparation(preparation); const r = runs.get(sessionId); if (!r) return; r.stopRequested = true; patchGeneration(r, { stopping: true }); if (r.acked) await cancelRun(r); },
      async cancelAll() { await Promise.all([...new Set([...runs.keys(), ...(preparation ? [preparation.sessionId] : [])])].map(sessionId => get().stopSession(sessionId))); },
      cancelOnLeave() { /* Route and inner-tab switches are not cancellation boundaries. */ },
      async decideTool(type) { const id = get().activeSessionId; const r = id ? runs.get(id) : undefined; const generation = id ? get().generations[id] : null; if (!r || !generation?.proposal || generation.proposalBusy || generation.stopping || generation.proposal.expires_at * 1000 <= Date.now()) return; patchGeneration(r, { proposalBusy: true }); try { await call<unknown>('advance_chat_tool_turn', { input: { task_id: r.taskId, action: { type, proposal_id: generation.proposal.id } } }); patchGeneration(r, { phase: type === 'approve' ? 'tool_running' : generation.phase, proposal: null, proposalBusy: false }); } catch (error) { if (runs.get(r.sessionId) !== r) return; const stale = error instanceof ChatCommandError && error.code === 1006017; patchGeneration(r, stale ? { proposal: null, proposalBusy: false } : { proposalBusy: false }); const text = chatErrorText(error); const code = error instanceof ChatCommandError ? error.code : null; set(state => ({ ...(state.activeSessionId === id ? { actionError: text, actionCode: code } : {}), sessionActionErrors: { ...state.sessionActionErrors, [id!]: text }, sessionActionCodes: { ...state.sessionActionCodes, [id!]: code } })); } },
      confirmDestination() { const pending = get().pendingDestination; if (!pending || get().activeSessionId !== pending.sessionId) return; set(state => ({ pendingDestination: null, sessionPendingDestinations: { ...state.sessionPendingDestinations, [pending.sessionId]: null } })); if (pending.kind === 'send') void get().send(pending.content, pending.tools, pending.fingerprint, pending.assetIds); else if (pending.kind === 'summary') void get().summarize(pending.fingerprint); else if (pending.kind === 'continue') void get().continueRun(pending.sourceTaskId, pending.intent, pending.tools, pending.fingerprint); else void get().regenerate(pending.messageId, pending.fingerprint); },
      dismissDestination(token) { const pending = get().pendingDestination; if (token && token !== pending) return; set(state => ({ pendingDestination: null, ...(pending ? { sessionPendingDestinations: { ...state.sessionPendingDestinations, [pending.sessionId]: null } } : {}) })); },
      dismissNotice() { const id = get().activeSessionId; if (id) set(state => ({ notice: null, sessionNotices: { ...state.sessionNotices, [id]: null } })); },
      reset() { ++accountEpoch; clearTimeout(searchTimer); searchTimer = undefined; preparation = null; uncertainRuns.clear(); runs.clear(); messageTokens.clear(); ++sessionsToken; ++searchToken; set(initialData()); },
    };
  });
}

export const useChatStore = createChatStore();

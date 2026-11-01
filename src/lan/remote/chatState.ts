import type { ChatMessage, ChatSession, ChatStreamEvent, SendAck } from './api';

// Chat state of the remote SPA. Messages are snapshots from the host; text streamed over the WebSocket lives in
// `live` (keyed by assistant message id) so it can arrive before the HTTP acknowledgement and survive a reload.

export interface LiveText {
  content: string;
  reasoning: string;
}

export interface FailureInfo {
  errorCode: string | number | null;
  providerCode: number | null;
  resultUnknown: boolean;
}

export interface ChatState {
  sessions: ChatSession[];
  messages: Record<string, ChatMessage[]>;
  /** Cursor for "load earlier" per session; null = nothing older, undefined = not loaded yet. */
  nextBefore: Record<string, number | null | undefined>;
  live: Record<string, LiveText>;
  failures: Record<string, FailureInfo>;
}

export const initialChatState: ChatState = { sessions: [], messages: {}, nextBefore: {}, live: {}, failures: {} };

export type ChatAction =
  | { type: 'sessions'; sessions: ChatSession[] }
  | { type: 'sessionCreated'; session: ChatSession }
  | { type: 'messages'; sessionId: string; items: ChatMessage[]; nextBeforeSeq: number | null; latest: boolean }
  | { type: 'sent'; sessionId: string; content: string; ack: SendAck; now: number }
  | { type: 'regenerated'; sessionId: string; messageId: string; ack: SendAck; now: number }
  | { type: 'event'; event: ChatStreamEvent };

function bySeq(a: ChatMessage, b: ChatMessage): number {
  return a.seq - b.seq;
}

function upsert(list: ChatMessage[], message: ChatMessage): ChatMessage[] {
  return [...list.filter((m) => m.id !== message.id), message].sort(bySeq);
}

function omit<T>(record: Record<string, T>, key: string): Record<string, T> {
  const { [key]: _removed, ...rest } = record;
  return rest;
}

function maxSeq(list: ChatMessage[]): number {
  return list.reduce((max, m) => Math.max(max, m.seq), 0);
}

function sessionOfMessage(state: ChatState, messageId: string): string | null {
  for (const [sessionId, list] of Object.entries(state.messages)) {
    if (list.some((m) => m.id === messageId)) return sessionId;
  }
  return null;
}

function placeholder(
  id: string,
  sessionId: string,
  seq: number,
  role: ChatMessage['role'],
  content: string,
  taskId: string | null,
  now: number,
): ChatMessage {
  return {
    id,
    session_id: sessionId,
    seq,
    role,
    content,
    status: role === 'user' ? 'completed' : 'generating',
    task_id: taskId,
    finish_reason: null,
    error_code: null,
    total_tokens: null,
    result_unknown: false,
    created_at: now,
    updated_at: now,
  };
}

function reduceMessages(state: ChatState, action: Extract<ChatAction, { type: 'messages' }>): ChatState {
  const { sessionId, items, nextBeforeSeq, latest } = action;
  const existing = state.messages[sessionId] ?? [];
  const incomingIds = new Set(items.map((m) => m.id));
  let merged: ChatMessage[];
  let keptOlder = false;
  if (latest) {
    // The incoming page is authoritative for its seq range. Messages outside it stay: older pages the user already
    // loaded, and newer local ones the host has not returned yet (the page was fetched before they were sent).
    const min = items.length ? items[0].seq : Infinity;
    const max = items.length ? items[items.length - 1].seq : -Infinity;
    const kept = existing.filter((m) => !incomingIds.has(m.id) && (m.seq > max || m.seq < min));
    keptOlder = kept.some((m) => m.seq < min);
    merged = [...kept, ...items].sort(bySeq);
  } else {
    merged = [...existing.filter((m) => !incomingIds.has(m.id)), ...items].sort(bySeq);
  }

  let live = state.live;
  if (latest) {
    live = { ...live };
    for (const message of items) {
      if (message.role !== 'assistant') continue;
      if (message.status === 'generating') {
        const current = live[message.id];
        // Keep whichever text is longer: events may have arrived while the page was in flight.
        live[message.id] =
          current && current.content.length > message.content.length
            ? current
            : { content: message.content, reasoning: current?.reasoning ?? '' };
      } else {
        delete live[message.id];
      }
    }
  }

  const nextBefore =
    latest && keptOlder ? state.nextBefore[sessionId] : nextBeforeSeq;
  return {
    ...state,
    messages: { ...state.messages, [sessionId]: merged },
    nextBefore: { ...state.nextBefore, [sessionId]: nextBefore },
    live,
  };
}

function reduceEvent(state: ChatState, event: ChatStreamEvent): ChatState {
  switch (event.kind) {
    case 'started': {
      const sessionId = sessionOfMessage(state, event.assistant_message_id);
      if (!sessionId) return state;
      const list = state.messages[sessionId].map((m) =>
        m.id === event.assistant_message_id && !m.task_id ? { ...m, task_id: event.task_id } : m,
      );
      return { ...state, messages: { ...state.messages, [sessionId]: list } };
    }
    case 'delta':
    case 'reasoning_delta': {
      const current = state.live[event.assistant_message_id] ?? { content: '', reasoning: '' };
      const next =
        event.kind === 'delta'
          ? { ...current, content: current.content + event.text }
          : { ...current, reasoning: current.reasoning + event.text };
      return { ...state, live: { ...state.live, [event.assistant_message_id]: next } };
    }
    case 'done': {
      const { message } = event;
      const list = upsert(state.messages[message.session_id] ?? [], message);
      return {
        ...state,
        messages: { ...state.messages, [message.session_id]: list },
        live: omit(state.live, message.id),
        failures: omit(state.failures, message.id),
      };
    }
    case 'error': {
      const id = event.assistant_message_id;
      const failures = {
        ...state.failures,
        [id]: { errorCode: event.error_code, providerCode: event.provider_code, resultUnknown: event.result_unknown },
      };
      if (event.message) {
        const list = upsert(state.messages[event.message.session_id] ?? [], event.message);
        return {
          ...state,
          messages: { ...state.messages, [event.message.session_id]: list },
          live: omit(state.live, id),
          failures,
        };
      }
      const sessionId = sessionOfMessage(state, id);
      const messages = sessionId
        ? { ...state.messages, [sessionId]: state.messages[sessionId].filter((m) => m.id !== id) }
        : state.messages;
      return { ...state, messages, live: omit(state.live, id), failures };
    }
  }
}

export function chatReducer(state: ChatState, action: ChatAction): ChatState {
  switch (action.type) {
    case 'sessions':
      return { ...state, sessions: [...action.sessions].sort((a, b) => b.updated_at - a.updated_at) };
    case 'sessionCreated':
      return {
        ...state,
        sessions: [action.session, ...state.sessions.filter((s) => s.id !== action.session.id)],
        messages: { ...state.messages, [action.session.id]: [] },
        nextBefore: { ...state.nextBefore, [action.session.id]: null },
      };
    case 'messages':
      return reduceMessages(state, action);
    case 'sent': {
      const { sessionId, content, ack, now } = action;
      let list = state.messages[sessionId] ?? [];
      const assistant = list.find((m) => m.id === ack.assistant_message_id);
      const base = maxSeq(list);
      if (!list.some((m) => m.id === ack.user_message_id)) {
        const seq = assistant ? assistant.seq - 1 : base + 1;
        list = upsert(list, placeholder(ack.user_message_id, sessionId, seq, 'user', content, null, now));
      }
      if (!assistant) {
        list = upsert(
          list,
          placeholder(ack.assistant_message_id, sessionId, maxSeq(list) + 1, 'assistant', '', ack.task_id, now),
        );
      }
      return {
        ...state,
        messages: { ...state.messages, [sessionId]: list },
        sessions: state.sessions.map((s) => (s.id === sessionId ? { ...s, updated_at: now } : s)),
      };
    }
    case 'regenerated': {
      const { sessionId, messageId, ack, now } = action;
      let list = state.messages[sessionId] ?? [];
      const old = list.find((m) => m.id === messageId);
      const seq = old ? old.seq : maxSeq(list) + 1;
      if (ack.assistant_message_id !== messageId) list = list.filter((m) => m.id !== messageId);
      const fresh = placeholder(ack.assistant_message_id, sessionId, seq, 'assistant', '', ack.task_id, now);
      const live = omit(state.live, messageId);
      return {
        ...state,
        messages: { ...state.messages, [sessionId]: upsert(list, fresh) },
        live,
        failures: omit(state.failures, messageId),
      };
    }
    case 'event':
      return reduceEvent(state, action.event);
  }
}

/** Text to show for a message: while generating, the streamed text wins over the stored snapshot if it is longer. */
export function displayContent(message: ChatMessage, live: LiveText | undefined): string {
  if (message.status !== 'generating' || !live) return message.content;
  return live.content.length >= message.content.length ? live.content : message.content;
}

export function findGenerating(messages: ChatMessage[]): ChatMessage | undefined {
  return messages.find((m) => m.role === 'assistant' && m.status === 'generating');
}

export function lastAssistant(messages: ChatMessage[]): ChatMessage | undefined {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i].role === 'assistant') return messages[i];
  }
  return undefined;
}

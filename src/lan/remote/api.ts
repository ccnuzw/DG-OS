// HTTP client of the remote SPA (interface contract §14). Identity is the HttpOnly cookie `dgos_lan`, which the
// browser attaches; this code never reads or stores it, and uses no web storage.

export interface LanSessionStatus {
  short_id: string;
  device_label: string;
  created_at: number;
  expires_at: number;
  idle_expires_at: number;
}

export interface LanTargets {
  providers: { id: string; name: string; models: { id: string; name: string }[] }[];
  skills: { id: string; name: string; version: string }[];
}

export interface ChatSession {
  id: string;
  title: string;
  title_source: 'auto' | 'user';
  provider_id: string | null;
  model_id: string | null;
  skills: { skill_id: string; version_id: string; content_hash: string }[];
  generation: { temperature?: number | null; reasoning?: { effort?: string; budget_tokens?: number } | null; max_tokens?: number | null; instruction?: string };
  no_skill_confirmed: boolean;
  created_at: number;
  updated_at: number;
}

export type ChatMessageStatus = 'generating' | 'completed' | 'interrupted' | 'failed';

export interface ChatMessage {
  id: string;
  session_id: string;
  seq: number;
  role: 'user' | 'assistant';
  content: string;
  status: ChatMessageStatus;
  task_id: string | null;
  finish_reason: string | null;
  error_code: string | null;
  /** null = unknown; never display 0. */
  total_tokens: number | null;
  result_unknown: boolean;
  created_at: number;
  updated_at: number;
}

export interface ChatMessagePage {
  items: ChatMessage[];
  next_before_seq: number | null;
}

export interface SendAck {
  task_id: string;
  user_message_id: string;
  assistant_message_id: string;
  dropped_turns: number;
  replayed: boolean;
}

export interface LanTarget {
  provider_id: string;
  model_id: string;
  skill_id: string | null;
  no_skill: boolean;
}

export type ChatStreamEvent =
  | { kind: 'started'; task_id: string; assistant_message_id: string }
  | { kind: 'delta'; assistant_message_id: string; text: string }
  | { kind: 'reasoning_delta'; assistant_message_id: string; text: string }
  | { kind: 'done'; message: ChatMessage }
  | {
      kind: 'error';
      assistant_message_id: string;
      message: ChatMessage | null;
      error_code: string | number | null;
      provider_code: number | null;
      result_unknown: boolean;
      regeneration_discarded: boolean;
    };

/** The server answered with a non-2xx status. `code` is 0 for transport errors, which have an empty body. */
export class LanApiError extends Error {
  readonly status: number;
  readonly code: number;
  readonly detail: Record<string, unknown> | null;

  constructor(status: number, code: number, message: string, detail: Record<string, unknown> | null) {
    super(message);
    this.name = 'LanApiError';
    this.status = status;
    this.code = code;
    this.detail = detail;
  }
}

/** fetch itself failed (no route to the host, connection reset, ...). The request may or may not have arrived. */
export class LanNetworkError extends Error {
  constructor() {
    super('network error');
    this.name = 'LanNetworkError';
  }
}

export const SESSION_GONE_CODE = 1010006;

/** True when a failed send/regenerate may still have been executed by the host, so a retry must reuse its id. */
export function isAmbiguousFailure(error: unknown): boolean {
  if (error instanceof LanNetworkError) return true;
  return error instanceof LanApiError && (error.status >= 500 || error.code === 0);
}

interface RequestOptions {
  method?: 'GET' | 'POST';
  body?: unknown;
  /** Report a 1010006 to `onSessionLost`. GET /session turns it into a plain 'unauthenticated' result instead. */
  notifyLost?: boolean;
}

export interface LanApi {
  pair(code: string): Promise<LanSessionStatus>;
  getSession(): Promise<LanSessionStatus>;
  logout(): Promise<void>;
  getTargets(): Promise<LanTargets>;
  createChatSession(target: LanTarget): Promise<ChatSession>;
  listChatSessions(): Promise<ChatSession[]>;
  listMessages(sessionId: string, beforeSeq?: number | null, limit?: number): Promise<ChatMessagePage>;
  sendMessage(sessionId: string, content: string, clientRequestId: string): Promise<SendAck>;
  regenerate(messageId: string, clientRequestId: string): Promise<SendAck>;
  cancelTask(taskId: string): Promise<boolean>;
}

export function createLanApi(onSessionLost: () => void): LanApi {
  async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const { method = 'GET', body, notifyLost = true } = options;
    const init: RequestInit = { method, credentials: 'same-origin', cache: 'no-store' };
    if (body !== undefined) {
      init.headers = { 'Content-Type': 'application/json' };
      init.body = JSON.stringify(body);
    }
    let response: Response;
    try {
      response = await fetch(path, init);
    } catch {
      throw new LanNetworkError();
    }
    if (!response.ok) {
      let code = 0;
      let message = '';
      let detail: Record<string, unknown> | null = null;
      try {
        const parsed: unknown = await response.json();
        if (parsed && typeof parsed === 'object') {
          if ('code' in parsed && typeof parsed.code === 'number') code = parsed.code;
          if ('message' in parsed && typeof parsed.message === 'string') message = parsed.message;
          if ('detail' in parsed && parsed.detail && typeof parsed.detail === 'object') {
            detail = Object.fromEntries(Object.entries(parsed.detail));
          }
        }
      } catch {
        // Transport errors (400/404/405/413/415) have an empty body.
      }
      if (code === SESSION_GONE_CODE && notifyLost) onSessionLost();
      throw new LanApiError(response.status, code, message, detail);
    }
    if (response.status === 204) return undefined as T;
    try {
      return (await response.json()) as T;
    } catch {
      throw new LanApiError(response.status, 0, '', null);
    }
  }

  const base = '/api/v1/lan';
  return {
    pair: (code) => request(`${base}/pair`, { method: 'POST', body: { code }, notifyLost: false }),
    getSession: () => request(`${base}/session`, { notifyLost: false }),
    logout: () => request(`${base}/logout`, { method: 'POST', notifyLost: false }),
    getTargets: () => request(`${base}/targets`),
    createChatSession: (target) => request(`${base}/chat/sessions`, { method: 'POST', body: { target } }),
    listChatSessions: async () => (await request<{ items: ChatSession[] }>(`${base}/chat/sessions`)).items,
    listMessages: (sessionId, beforeSeq, limit = 50) => {
      const query = new URLSearchParams({ limit: String(limit) });
      if (beforeSeq !== undefined && beforeSeq !== null) query.set('before_seq', String(beforeSeq));
      return request(`${base}/chat/sessions/${encodeURIComponent(sessionId)}/messages?${query.toString()}`);
    },
    sendMessage: (sessionId, content, clientRequestId) =>
      request(`${base}/chat/sessions/${encodeURIComponent(sessionId)}/messages`, {
        method: 'POST',
        body: { content, client_request_id: clientRequestId },
      }),
    regenerate: (messageId, clientRequestId) =>
      request(`${base}/chat/messages/${encodeURIComponent(messageId)}/regenerate`, {
        method: 'POST',
        body: { client_request_id: clientRequestId },
      }),
    cancelTask: async (taskId) =>
      (await request<{ cancelled: boolean }>(`${base}/chat/tasks/${encodeURIComponent(taskId)}/cancel`, { method: 'POST' }))
        .cancelled,
  };
}

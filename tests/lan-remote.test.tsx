import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import type { Root } from "react-dom/client";
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../src/lan/remote/App';
import { mountLanApp } from '../src/lan/remote/boot';
import { setRemoteLocale } from '../src/lan/remote/i18n';

// Remote LAN SPA (V1-FR-010): fetch and WebSocket are replaced by fakes; nothing touches a network.

const BASE = '/api/v1/lan';
const CODE = '0123456789ABCDEFGHJKMNPQRS';
const CODE_WITH_DASHES = '0123-4567-89AB-CDEF-GHJK-MNPQ-RS';
const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const SESSION_STATUS = { short_id: 'ab12cd34', device_label: 'iPhone Safari', created_at: 1, expires_at: 2, idle_expires_at: 3 };
const CHAT_SESSION = {
  id: 's1',
  title: 'Chat A',
  title_source: 'auto',
  provider_id: 'p1',
  model_id: 'm1',
  skill_id: null,
  skill_version_id: null,
  no_skill_confirmed: true,
  created_at: 1,
  updated_at: 2,
};
const ACK = { task_id: 'task-1', user_message_id: 'u1', assistant_message_id: 'a1', dropped_turns: 0, replayed: false };

function message(overrides: Record<string, unknown>) {
  return {
    id: 'a1',
    session_id: 's1',
    seq: 2,
    role: 'assistant',
    content: '',
    status: 'completed',
    task_id: 'task-1',
    finish_reason: 'stop',
    error_code: null,
    total_tokens: null,
    result_unknown: false,
    created_at: 1,
    updated_at: 2,
    ...overrides,
  };
}

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}
function failure(status: number, code: number, detail?: Record<string, unknown>) {
  return json({ code, message: 'fixed text', ...(detail ? { detail } : {}) }, status);
}

interface Call {
  method: string;
  path: string;
  search: string;
  body: Record<string, unknown> | null;
}
type Handler = (call: Call) => Response | Promise<Response>;

let calls: Call[];
let routes: Record<string, Handler>;
let order: string[];

function defaultRoutes(): Record<string, Handler> {
  return {
    [`GET ${BASE}/bootstrap`]: () => json({ locale: 'zh-CN', catalog_version: 'v1' }),
    [`GET ${BASE}/session`]: () => json(SESSION_STATUS),
    [`POST ${BASE}/pair`]: () => json(SESSION_STATUS),
    [`POST ${BASE}/logout`]: () => new Response(null, { status: 204 }),
    [`GET ${BASE}/targets`]: () =>
      json({
        providers: [{ id: 'p1', name: 'Provider One', models: [{ id: 'm1', name: 'Model One' }] }],
        skills: [{ id: 'k1', name: 'Skill One', version: '1.0.0' }],
      }),
    [`POST ${BASE}/chat/sessions`]: () => json(CHAT_SESSION, 201),
    [`GET ${BASE}/chat/sessions`]: () => json({ items: [CHAT_SESSION] }),
    [`GET ${BASE}/chat/sessions/s1/messages`]: () => json({ items: [], next_before_seq: null }),
    [`POST ${BASE}/chat/sessions/s1/messages`]: () => json(ACK, 202),
  };
}

class FakeWebSocket {
  static instances: FakeWebSocket[] = [];
  static get latest(): FakeWebSocket {
    return FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
  }
  onopen: ((event: Event) => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  onclose: ((event: CloseEvent) => void) | null = null;
  onerror: ((event: Event) => void) | null = null;
  sent: unknown[] = [];
  closedByClient = false;
  readonly url: string;
  constructor(url: string) {
    this.url = url;
    FakeWebSocket.instances.push(this);
  }
  send(data: unknown) {
    this.sent.push(data);
  }
  close() {
    this.closedByClient = true;
  }
  open() {
    this.onopen?.(new Event('open'));
  }
  frame(value: unknown) {
    this.onmessage?.(new MessageEvent('message', { data: JSON.stringify(value) }));
  }
  serverClose(code: number) {
    this.onclose?.(new CloseEvent('close', { code }));
  }
}

async function socketDo(action: (socket: FakeWebSocket) => void) {
  await act(async () => {
    action(FakeWebSocket.latest);
  });
}

function callsTo(method: string, path: string): Call[] {
  return calls.filter((c) => c.method === method && c.path === path);
}

/** Renders the app with a valid session and an open stream. */
async function enterChat() {
  render(<App />);
  await screen.findByLabelText('消息内容');
  await socketDo((s) => s.open());
}

async function typeAndSend(text: string) {
  fireEvent.change(screen.getByLabelText('消息内容'), { target: { value: text } });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
  });
}

beforeEach(() => {
  setRemoteLocale('zh-CN');
  calls = [];
  order = [];
  routes = defaultRoutes();
  FakeWebSocket.instances = [];
  vi.stubGlobal('WebSocket', FakeWebSocket);
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input), location.href);
      const method = init?.method ?? 'GET';
      const call: Call = {
        method,
        path: url.pathname,
        search: url.search,
        body: typeof init?.body === 'string' ? (JSON.parse(init.body) as Record<string, unknown>) : null,
      };
      calls.push(call);
      order.push(`fetch ${method} ${url.pathname}`);
      const handler = routes[`${method} ${url.pathname}`];
      return handler ? handler(call) : new Response(null, { status: 404 });
    }),
  );
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  history.replaceState(null, '', '/');
});

describe('pairing from the URL fragment', () => {
  it('removes the fragment with replaceState before any request and pairs once', async () => {
    const original = history.replaceState.bind(history);
    vi.spyOn(history, 'replaceState').mockImplementation((state, title, url) => {
      order.push('replaceState');
      original(state, title, url);
    });
    history.replaceState(null, '', `/?x=1#code=${CODE_WITH_DASHES.toLowerCase()}`);
    order.length = 0;
    expect(location.hash).toContain('code=');

    const container = document.createElement('div');
    document.body.appendChild(container);
    let root!: Root;
    await act(async () => {
      root = mountLanApp(container);
    });

    await screen.findByLabelText('消息内容');
    expect(order[0]).toBe('replaceState');
    expect(location.hash).toBe('');
    expect(location.pathname + location.search).toBe('/?x=1');
    expect(order).toContain(`fetch GET ${BASE}/bootstrap`);
    expect(order).toContain(`fetch POST ${BASE}/pair`);
    expect(callsTo('POST', `${BASE}/pair`)).toHaveLength(1);
    expect(callsTo('POST', `${BASE}/pair`)[0].body).toEqual({ code: CODE.toLowerCase().toUpperCase() });
    expect(callsTo('GET', `${BASE}/session`)).toHaveLength(0);

    await act(async () => root.unmount());
    container.remove();
  });

  it('checks GET /session when there is no fragment and shows pairing on 401', async () => {
    routes[`GET ${BASE}/session`] = () => failure(401, 1010006);
    render(<App />);
    expect(await screen.findByRole('heading', { name: '连接到 DG-OS' })).toBeTruthy();
    expect(callsTo('GET', `${BASE}/session`)).toHaveLength(1);
    expect(callsTo('POST', `${BASE}/pair`)).toHaveLength(0);
  });
});

describe('pairing view', () => {
  async function openPairing() {
    routes[`GET ${BASE}/session`] = () => failure(401, 1010006);
    render(<App />);
    await screen.findByLabelText('访问凭据');
  }
  async function submit(value: string) {
    fireEvent.change(screen.getByLabelText('访问凭据'), { target: { value } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '连接' }));
    });
  }

  it('shows the message for 1010004', async () => {
    await openPairing();
    routes[`POST ${BASE}/pair`] = () => failure(401, 1010004);
    await submit(CODE_WITH_DASHES);
    expect(await screen.findByText('访问凭据无效或已使用')).toBeTruthy();
  });

  it('shows retry_after_seconds for 1010011 pair_ip', async () => {
    await openPairing();
    routes[`POST ${BASE}/pair`] = () => failure(429, 1010011, { scope: 'pair_ip', retry_after_seconds: 37 });
    await submit(CODE);
    expect(await screen.findByText('尝试过于频繁，请 37 秒后再试')).toBeTruthy();
  });

  it('maps the other pairing errors', async () => {
    await openPairing();
    routes[`POST ${BASE}/pair`] = () => failure(401, 1010005);
    await submit(CODE);
    expect(await screen.findByText('访问凭据已过期，请在主机上重新生成')).toBeTruthy();
    routes[`POST ${BASE}/pair`] = () => failure(429, 1010011, { scope: 'session_limit' });
    await submit(CODE);
    expect(await screen.findByText('已连接设备过多，请在主机上断开不用的设备')).toBeTruthy();
    routes[`POST ${BASE}/pair`] = () => failure(403, 1010007);
    await submit(CODE);
    expect(await screen.findByText('当前地址不被允许')).toBeTruthy();
  });

  it('strips dashes and spaces, uppercases, and does not send a code of the wrong length', async () => {
    await openPairing();
    await submit('0123 4567-89ab cdef-ghjk-mnpq-rs');
    await screen.findByLabelText('消息内容');
    expect(callsTo('POST', `${BASE}/pair`)[0].body).toEqual({ code: CODE });
    cleanup();

    calls.length = 0;
    await openPairing();
    await submit('ABCD');
    expect(await screen.findByText('访问凭据应为 26 位字符')).toBeTruthy();
    expect(callsTo('POST', `${BASE}/pair`)).toHaveLength(0);
  });
});

describe('sending', () => {
  it('uses a UUID v4 made without crypto.randomUUID and reuses it when the send is retried after a network error', async () => {
    const randomUUID = vi.fn();
    vi.stubGlobal('crypto', { getRandomValues: globalThis.crypto.getRandomValues.bind(globalThis.crypto), randomUUID });
    await enterChat();

    let attempts = 0;
    routes[`POST ${BASE}/chat/sessions/s1/messages`] = () => {
      attempts += 1;
      if (attempts === 1) throw new TypeError('Failed to fetch');
      return json(ACK, 202);
    };
    await typeAndSend('问候');
    expect(await screen.findByText('无法连接到主机，请检查网络后重试')).toBeTruthy();
    expect((screen.getByLabelText('消息内容') as HTMLTextAreaElement).value).toBe('问候');

    await typeAndSend('问候');
    await screen.findByText('正在思考…');

    const sends = callsTo('POST', `${BASE}/chat/sessions/s1/messages`);
    expect(sends).toHaveLength(2);
    const first = sends[0].body?.client_request_id as string;
    expect(first).toMatch(UUID_V4);
    expect(sends[1].body?.client_request_id).toBe(first);
    expect(sends[1].body?.content).toBe('问候');
    expect(randomUUID).not.toHaveBeenCalled();
  });

  it('uses a new id after the draft changed', async () => {
    await enterChat();
    routes[`POST ${BASE}/chat/sessions/s1/messages`] = () => {
      throw new TypeError('Failed to fetch');
    };
    await typeAndSend('第一条');
    await screen.findByText('无法连接到主机，请检查网络后重试');
    await typeAndSend('第二条');
    await waitFor(() => expect(callsTo('POST', `${BASE}/chat/sessions/s1/messages`)).toHaveLength(2));
    const [a, b] = callsTo('POST', `${BASE}/chat/sessions/s1/messages`);
    expect(a.body?.client_request_id).not.toBe(b.body?.client_request_id);
  });

  it.each([
    [409, 1006012, 'Provider 地址已变化，请在主机上确认后再发送'],
    [409, 1006010, '正在生成中'],
    [409, 1006013, '技能不可用'],
  ])('shows the text for %i %i', async (status, code, text) => {
    await enterChat();
    routes[`POST ${BASE}/chat/sessions/s1/messages`] = () => failure(status, code);
    await typeAndSend('问候');
    expect(await screen.findByText(text)).toBeTruthy();
  });

  it('goes back to pairing on a 401 with code 1010006', async () => {
    await enterChat();
    routes[`POST ${BASE}/chat/sessions/s1/messages`] = () => failure(401, 1010006);
    await typeAndSend('问候');
    expect(await screen.findByRole('heading', { name: '连接到 DG-OS' })).toBeTruthy();
    expect(screen.getByText('已被主机断开连接')).toBeTruthy();
  });

  it('cancels the running task', async () => {
    await enterChat();
    routes[`POST ${BASE}/chat/tasks/task-1/cancel`] = () => json({ cancelled: true });
    await typeAndSend('问候');
    await act(async () => {
      fireEvent.click(await screen.findByRole('button', { name: '停止生成' }));
    });
    expect(callsTo('POST', `${BASE}/chat/tasks/task-1/cancel`)).toHaveLength(1);
  });

  it('regenerates the last assistant reply with a new request id and shows 1006014', async () => {
    routes[`GET ${BASE}/chat/sessions/s1/messages`] = () =>
      json({
        items: [message({ id: 'u0', seq: 1, role: 'user', content: '问' }), message({ id: 'a0', seq: 2, content: '答' })],
        next_before_seq: null,
      });
    await enterChat();
    routes[`POST ${BASE}/chat/messages/a0/regenerate`] = () => failure(409, 1006014);
    await act(async () => {
      fireEvent.click(await screen.findByRole('button', { name: '重新生成' }));
    });
    expect(await screen.findByText('只能重新生成最后一条回复')).toBeTruthy();
    const body = callsTo('POST', `${BASE}/chat/messages/a0/regenerate`)[0].body;
    expect(body?.client_request_id).toMatch(UUID_V4);
  });
});

describe('streaming', () => {
  const reply = (extra: Record<string, unknown> = {}) => message({ content: '最终回答', ...extra });

  it('appends delta frames to the generating message and replaces them with the done snapshot', async () => {
    await enterChat();
    await typeAndSend('问候');
    await screen.findByText('正在思考…');

    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'started', task_id: 'task-1', assistant_message_id: 'a1' } }));
    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'delta', assistant_message_id: 'a1', text: '部分' } }));
    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'delta', assistant_message_id: 'a1', text: '回答' } }));
    expect(screen.getByText('部分回答')).toBeTruthy();
    await socketDo((s) =>
      s.frame({ type: 'chat', event: { kind: 'reasoning_delta', assistant_message_id: 'a1', text: '先想一想' } }),
    );
    expect(screen.getByText('先想一想')).toBeTruthy();

    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'done', message: reply() } }));
    expect(screen.getByText('最终回答')).toBeTruthy();
    expect(screen.queryByText('部分回答')).toBeNull();
    expect(screen.queryByText('先想一想')).toBeNull();
    expect(screen.queryByRole('button', { name: '停止生成' })).toBeNull();
  });

  it('shows token usage only when known and never shows 0', async () => {
    await enterChat();
    await typeAndSend('问候');
    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'done', message: reply({ total_tokens: null }) } }));
    expect(screen.queryByText(/tokens/)).toBeNull();

    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'done', message: reply({ total_tokens: 0 }) } }));
    expect(screen.queryByText(/tokens/)).toBeNull();

    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'done', message: reply({ total_tokens: 42 }) } }));
    expect(screen.getByText('42 tokens')).toBeTruthy();
  });

  it('replaces the partial text with the failed snapshot on an error frame', async () => {
    await enterChat();
    await typeAndSend('问候');
    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'delta', assistant_message_id: 'a1', text: '半截' } }));
    await socketDo((s) =>
      s.frame({
        type: 'chat',
        event: {
          kind: 'error',
          assistant_message_id: 'a1',
          message: message({ content: '半截已保存', status: 'interrupted', error_code: 'stream_interrupted' }),
          error_code: 'stream_interrupted',
          provider_code: 1001007,
          result_unknown: true,
          regeneration_discarded: false,
        },
      }),
    );
    expect(screen.getByText('半截已保存')).toBeTruthy();
    expect(screen.getByText('生成已中断（1001007）')).toBeTruthy();
    expect(screen.getByText('结果未知，上游可能已产生用量')).toBeTruthy();
  });

  it('renders content as text, not HTML', async () => {
    await enterChat();
    await typeAndSend('问候');
    await socketDo((s) =>
      s.frame({ type: 'chat', event: { kind: 'done', message: reply({ content: '<img src=x onerror=alert(1)>' }) } }),
    );
    expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeTruthy();
    expect(document.querySelector('img')).toBeNull();
  });

  it('never sends a data frame and connects to /api/v1/lan/stream', async () => {
    await enterChat();
    await typeAndSend('问候');
    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'delta', assistant_message_id: 'a1', text: 'x' } }));
    expect(FakeWebSocket.latest.url).toBe(`ws://${location.host}${BASE}/stream`);
    for (const socket of FakeWebSocket.instances) expect(socket.sent).toEqual([]);
  });
});

describe('connection lifecycle', () => {
  it('returns to pairing with a notice on a session_ended frame', async () => {
    await enterChat();
    await socketDo((s) => s.frame({ type: 'session_ended', reason: 'host_disconnect' }));
    expect(await screen.findByRole('heading', { name: '连接到 DG-OS' })).toBeTruthy();
    expect(screen.getByText('已被主机断开连接')).toBeTruthy();
  });

  it('returns to pairing on close code 4401', async () => {
    await enterChat();
    await socketDo((s) => s.serverClose(4401));
    expect(await screen.findByRole('heading', { name: '连接到 DG-OS' })).toBeTruthy();
    expect(screen.getByText('已被主机断开连接')).toBeTruthy();
  });

  it('says the session expired for an expiry reason', async () => {
    await enterChat();
    await socketDo((s) => s.frame({ type: 'session_ended', reason: 'idle_expired' }));
    expect(await screen.findByText('连接已过期，请在主机上重新生成访问凭据')).toBeTruthy();
  });

  it('stops reconnecting on close code 4409 and offers a manual reconnect', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await enterChat();
    const sockets = FakeWebSocket.instances.length;
    const probes = callsTo('GET', `${BASE}/session`).length;

    await socketDo((s) => s.serverClose(4409));
    expect(await screen.findByText('已在其他标签页打开')).toBeTruthy();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(FakeWebSocket.instances).toHaveLength(sockets);
    expect(callsTo('GET', `${BASE}/session`)).toHaveLength(probes);

    const messageLoads = callsTo('GET', `${BASE}/chat/sessions/s1/messages`).length;
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '重新连接' }));
    });
    expect(FakeWebSocket.instances).toHaveLength(sockets + 1);
    await socketDo((s) => s.open());
    // The first open after a manual restart resyncs: the list was not kept up to date while stopped.
    expect(callsTo('GET', `${BASE}/chat/sessions/s1/messages`).length).toBe(messageLoads + 1);
    expect(screen.queryByText('已在其他标签页打开')).toBeNull();
  });

  it('shows the host-stopped text on close code 1001 without reconnecting', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await enterChat();
    const sockets = FakeWebSocket.instances.length;
    await socketDo((s) => s.serverClose(1001));
    expect(await screen.findByText('主机已停止服务')).toBeTruthy();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(FakeWebSocket.instances).toHaveLength(sockets);
  });

  it('after an abnormal close it checks the session, reconnects with backoff and reloads the messages', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await enterChat();
    const sockets = FakeWebSocket.instances.length;
    const probes = callsTo('GET', `${BASE}/session`).length;
    const loads = callsTo('GET', `${BASE}/chat/sessions/s1/messages`).length;

    await socketDo((s) => s.serverClose(1006));
    expect(await screen.findByText('连接已断开，正在重新连接…')).toBeTruthy();
    await waitFor(() => expect(callsTo('GET', `${BASE}/session`)).toHaveLength(probes + 1));
    expect(FakeWebSocket.instances).toHaveLength(sockets);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(FakeWebSocket.instances).toHaveLength(sockets + 1);
    await socketDo((s) => s.open());
    await waitFor(() => expect(callsTo('GET', `${BASE}/chat/sessions/s1/messages`)).toHaveLength(loads + 1));
    expect(screen.queryByText('连接已断开，正在重新连接…')).toBeNull();
  });

  it('goes to pairing when the session check after an abnormal close answers 401', async () => {
    await enterChat();
    routes[`GET ${BASE}/session`] = () => failure(401, 1010006);
    await socketDo((s) => s.serverClose(1006));
    expect(await screen.findByRole('heading', { name: '连接到 DG-OS' })).toBeTruthy();
  });

  it('logs out and returns to pairing', async () => {
    await enterChat();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '退出' }));
    });
    expect(await screen.findByRole('heading', { name: '连接到 DG-OS' })).toBeTruthy();
    expect(callsTo('POST', `${BASE}/logout`)).toHaveLength(1);
  });
});

describe('sessions', () => {
  it('creates a session with a fixed target', async () => {
    routes[`GET ${BASE}/chat/sessions`] = () => json({ items: [] });
    render(<App />);
    await screen.findByText('开始聊天');
    await screen.findByRole('option', { name: 'Provider One' });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '开始聊天' }));
    });
    await screen.findByLabelText('消息内容');
    expect(callsTo('POST', `${BASE}/chat/sessions`)[0].body).toEqual({
      target: { provider_id: 'p1', model_id: 'm1', skill_id: null, no_skill: true },
    });
    expect(screen.queryByRole('button', { name: /重命名|删除/ })).toBeNull();
  });

  it('sends the chosen skill and shows the session limit text', async () => {
    routes[`GET ${BASE}/chat/sessions`] = () => json({ items: [] });
    routes[`POST ${BASE}/chat/sessions`] = () => failure(429, 1010011, { scope: 'chat_sessions' });
    render(<App />);
    await screen.findByRole('option', { name: 'Provider One' });
    fireEvent.change(screen.getByLabelText('技能'), { target: { value: 'k1' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '开始聊天' }));
    });
    expect(await screen.findByText('本次连接创建的会话已达上限（100）')).toBeTruthy();
    expect(callsTo('POST', `${BASE}/chat/sessions`)[0].body).toEqual({
      target: { provider_id: 'p1', model_id: 'm1', skill_id: 'k1', no_skill: false },
    });
  });

  it('loads earlier messages with next_before_seq', async () => {
    routes[`GET ${BASE}/chat/sessions/s1/messages`] = (call) =>
      call.search.includes('before_seq=5')
        ? json({ items: [message({ id: 'old', seq: 4, role: 'user', content: '更早的消息' })], next_before_seq: null })
        : json({ items: [message({ id: 'new', seq: 5, content: '较新的消息' })], next_before_seq: 5 });
    await enterChat();
    await screen.findByText('较新的消息');
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: '加载更早' }));
    });
    expect(await screen.findByText('更早的消息')).toBeTruthy();
    const requests = callsTo('GET', `${BASE}/chat/sessions/s1/messages`);
    expect(requests[0].search).toContain('limit=50');
    expect(requests[requests.length - 1].search).toContain('before_seq=5');
    expect(screen.queryByRole('button', { name: '加载更早' })).toBeNull();
  });
});

describe('no browser storage', () => {
  it('touches no web storage, IndexedDB or document.cookie through a full flow', async () => {
    const storage = ['getItem', 'setItem', 'removeItem', 'clear', 'key'].map((name) =>
      vi.spyOn(Storage.prototype, name as 'getItem'),
    );
    const cookieGet = vi.spyOn(Document.prototype, 'cookie', 'get');
    const cookieSet = vi.spyOn(Document.prototype, 'cookie', 'set');
    vi.stubGlobal(
      'indexedDB',
      new Proxy({}, {
        get() {
          throw new Error('indexedDB touched');
        },
      }),
    );

    history.replaceState(null, '', `/#code=${CODE}`);
    const container = document.createElement('div');
    document.body.appendChild(container);
    let root!: Root;
    await act(async () => {
      root = mountLanApp(container);
    });
    await screen.findByLabelText('消息内容');
    await socketDo((s) => s.open());
    await typeAndSend('问候');
    await socketDo((s) => s.frame({ type: 'chat', event: { kind: 'done', message: message({ content: '好' }) } }));
    await socketDo((s) => s.frame({ type: 'session_ended', reason: 'logout' }));

    for (const spy of storage) expect(spy).not.toHaveBeenCalled();
    expect(cookieGet).not.toHaveBeenCalled();
    expect(cookieSet).not.toHaveBeenCalled();
    await act(async () => root.unmount());
    container.remove();
  });
});

/** Comments may name the things the code must avoid; only code counts. */
function stripComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
}

describe('remote SPA sources', () => {
  const dir = join(__dirname, '..', 'src', 'lan', 'remote');
  const files = readdirSync(dir)
    .filter((name) => /\.(tsx?|css)$/.test(name))
    .map((name) => [name, stripComments(readFileSync(join(dir, name), 'utf8'))] as const);

  it.each([
    ['Tauri packages', /@tauri-apps|__TAURI/],
    ['web storage', /localStorage|sessionStorage|indexedDB/],
    ['cookie access', /document\.cookie/],
    ['randomUUID (undefined on http://192.168.x.x)', /randomUUID/],
    ['raw HTML rendering', /dangerouslySetInnerHTML|innerHTML/],
    ['the @dgos/ui barrel (it pulls in the Tauri appearance host)', /from\s+['"]@dgos\/ui['"]/],
    ['module host or module runtime', /\/modules\/|\.\.\/\.\.\/modules/],
  ])('does not use %s', (_label, pattern) => {
    for (const [name, source] of files) expect({ name, hit: pattern.test(source) }).toEqual({ name, hit: false });
  });

  it('has an entry HTML without inline script, inline style or style attributes', () => {
    const html = readFileSync(join(__dirname, '..', 'lan-web', 'index.html'), 'utf8');
    expect(html).not.toMatch(/<style/i);
    expect(html).not.toMatch(/\sstyle\s*=/i);
    expect(html).not.toMatch(/<script(?![^>]*\ssrc=)/i);
  });
});

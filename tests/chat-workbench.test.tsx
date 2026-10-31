/* eslint-disable @typescript-eslint/no-explicit-any */
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import Workbench from '../src/pages/Workbench';
import ChatWorkbench from '../src/components/chat/ChatWorkbench';
import { useChatStore } from '../src/stores/chatStore';
import { MockChannel } from './helpers/mockChannel';

// FR-006 chat front end (AC01-AC29 subset that needs the DOM). Contract: V1-接口契约 §10.1, V1-错误码 §3.6.
// Synthetic data only. The host is a table of command handlers; Channels are the controllable MockChannel.
// vi.mock is hoisted above the static imports, so the factory cannot reference them: the helper must be imported inside it.
vi.mock('@tauri-apps/api/core', async () => {
  const { MockChannel: Channel } = await import('./helpers/mockChannel');
  return { invoke: vi.fn(), Channel };
});

type Args = Record<string, any>;
type Handler = (args: Args) => unknown;
const mocked = vi.mocked(invoke);
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

const ok = (data: unknown) => ({ success: true, data });
const fail = (code: number, detail?: unknown) => ({ success: false, error: { code, message: 'host text must never be shown', detail } });

const session = (over: Args = {}) => ({
  id: 's1', title: '季度复盘', title_source: 'user', provider_id: 'p1', model_id: 'chat1', skill_id: null,
  skill_version_id: null, no_skill_confirmed: true, created_at: 1, updated_at: 2, ...over,
});
const message = (seq: number, role: 'user' | 'assistant', content: string, over: Args = {}) => ({
  id: `m${seq}`, session_id: 's1', seq, role, content, status: 'completed', task_id: null, finish_reason: 'stop',
  error_code: null, total_tokens: 10, result_unknown: false, created_at: 1, updated_at: 1, ...over,
});
const tool = (name: string, over: Args = {}) => ({
  server_id: 'srv1', tool_name: name, description: `合成工具 ${name}`, input_schema: {}, schema_hash: `hash-${name}`, discovered_at: 1, ...over,
});
const proposal = (over: Args = {}) => ({
  id: 'prop1', server_id: 'srv1', server_name: '文件服务', tool_name: 'read_file', schema_hash: 'hash-read_file',
  arguments: { path: '/synthetic/a.txt', limit: 3 }, expires_at: Math.floor(Date.now() / 1000) + 60, ...over,
});

let handlers: Record<string, Handler>;

const calls = (command: string): Args[] => mocked.mock.calls.filter(([name]) => name === command).map(([, args]) => (args ?? {}) as Args);

function keysOf(value: unknown, out: string[] = [], seen = new Set<unknown>()): string[] {
  if (typeof value !== 'object' || value === null || seen.has(value)) return out;
  seen.add(value);
  for (const [key, child] of Object.entries(value)) {
    out.push(key);
    keysOf(child, out, seen);
  }
  return out;
}

beforeEach(() => {
  MockChannel.reset();
  useChatStore.getState().reset();
  handlers = {
    list_chat_sessions: () => ok({ items: [session()], next_cursor: null }),
    list_chat_messages: () => ok({ items: [], next_before_seq: null }),
    cancel_chat_generation: () => ok(true),
    list_providers: () => ok([{ id: 'p1', name: '合成服务', enabled: true, credential_status: 'stored' }]),
    list_models: () => ok([{ id: 'mm1', model_id: 'chat1', provider_id: 'p1', model_name: '对话模型', model_type: 'chat' }]),
    list_installed_skills: () => [],
    list_mcp_servers: () => [{ id: 'srv1', name: '文件服务', enabled: true, status: 'ready' }],
    list_mcp_tools: () => [tool('read_file')],
    list_workbench_tasks: () => [],
  };
  mocked.mockImplementation(async (command, args) => {
    const handler = handlers[command];
    if (!handler) throw new Error(`Unexpected command: ${command}`);
    return handler((args ?? {}) as Args);
  });
});

afterEach(() => {
  cleanup();
  useChatStore.getState().reset();
  Reflect.deleteProperty(navigator, 'clipboard');
  Reflect.deleteProperty(document, 'execCommand');
});

// Identity comes from the host: no payload of any test may carry a user id (checked after every test).
afterEach(() => {
  for (const [, args] of mocked.mock.calls) {
    expect(keysOf(args).filter(key => /^user_?id$/i.test(key))).toEqual([]);
  }
});

function renderPage(page: 'chat' | 'workbench' = 'chat') {
  return render(<MemoryRouter>{page === 'chat' ? <ChatWorkbench /> : <Workbench />}</MemoryRouter>);
}

async function openSession(title = '季度复盘', page: 'chat' | 'workbench' = 'chat') {
  const view = renderPage(page);
  fireEvent.click(await screen.findByRole('button', { name: new RegExp(title) }));
  await screen.findByRole('textbox', { name: '消息输入' });
  return view;
}

const draftBox = () => screen.getByRole('textbox', { name: '消息输入' }) as HTMLTextAreaElement;
const typeDraft = (text: string) => fireEvent.change(draftBox(), { target: { value: text } });
const sendButton = () => screen.getByRole('button', { name: '发送' }) as HTMLButtonElement;

/** The send command stays pending until the returned function acknowledges it. */
function deferSend() {
  const { promise, resolve } = Promise.withResolvers<unknown>();
  handlers.send_chat_message = () => promise;
  return (ack: Args) => resolve(ok(ack));
}

async function sentArgs(count = 1) {
  await waitFor(() => expect(calls('send_chat_message')).toHaveLength(count));
  return calls('send_chat_message')[count - 1];
}

const ackFor = (args: Args, over: Args = {}) => ({
  task_id: args.input.client_request_id, user_message_id: 'u-new', assistant_message_id: 'a1', dropped_turns: 0, replayed: false, ...over,
});
const emit = (channel: MockChannel, event: Args) => act(() => { channel.emit(event); });
const settle = (resolveAck: (ack: Args) => void, ack: Args) => act(async () => { resolveAck(ack); });

async function choose(name: string, option: string) {
  const trigger = await screen.findByRole('combobox', { name });
  await waitFor(() => expect((trigger as HTMLButtonElement).disabled).toBe(false));
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  fireEvent.click(within(await screen.findByRole('listbox')).getByRole('option', { name: option }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}

async function pickTools(...names: RegExp[]) {
  const toggle = screen.getByRole('button', { name: /MCP 工具（可选/ });
  if (toggle.getAttribute('aria-expanded') !== 'true') fireEvent.click(toggle);
  for (const name of names) fireEvent.click(await screen.findByRole('checkbox', { name }));
}

const pickerLabel = (selected: number) => screen.getByRole('button', { name: new RegExp(`MCP 工具（可选，已选 ${selected} / 8）`) });

describe('sessions', () => {
  it('creating a session from the empty state opens it with the undecided target and skill explained', async () => {
    handlers.list_chat_sessions = () => ok({ items: [], next_cursor: null });
    handlers.create_chat_session = () => ok(session({ id: 'new1', title: '新会话', title_source: 'auto', provider_id: null, model_id: null, no_skill_confirmed: false }));
    renderPage();
    await screen.findByRole('heading', { name: '开始一次对话' });
    fireEvent.click(screen.getAllByRole('button', { name: '新建会话' })[0]);
    await screen.findByRole('textbox', { name: '消息输入' });
    expect(calls('create_chat_session')).toEqual([{ input: {} }]);
    expect(screen.getByRole('heading', { name: '新会话' })).toBeTruthy();
    expect(screen.getByText(/请先在上方选择 Provider 和模型/)).toBeTruthy();
    expect(screen.getByText(/这个会话还没有选择技能/)).toBeTruthy();
    typeDraft('先打个招呼');
    expect(sendButton().disabled).toBe(true);
    expect(calls('send_chat_message')).toHaveLength(0);
  });

  it('selecting another session loads its newest page of messages', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session({ id: 'sA', title: '会话 A' }), session({ id: 'sB', title: '会话 B' })], next_cursor: null });
    handlers.list_chat_messages = ({ input }) => ok({ items: [message(1, 'user', `属于 ${input.session_id}`, { session_id: input.session_id })], next_before_seq: null });
    renderPage();
    fireEvent.click(await screen.findByRole('button', { name: /会话 B/ }));
    await screen.findByText('属于 sB');
    expect(calls('list_chat_messages')).toEqual([{ input: { session_id: 'sB', limit: 50 } }]);
    expect(screen.getByRole('heading', { name: '会话 B' })).toBeTruthy();
  });

  // Ten jsdom render rounds up to 300 nodes: about 1.6 s alone, but over the 5 s default when the
  // machine is heavily loaded (seen at load average ~42). The test asserts no timing, so a larger
  // budget hides no defect.
  it('session list pages by cursor and stops at the 300-node cap', { timeout: 20_000 }, async () => {
    const pool = Array.from({ length: 400 }, (_, i) => session({ id: `s${i}`, title: `会话 ${i}`, updated_at: 1000 - i }));
    handlers.list_chat_sessions = ({ input }) => {
      const start = input.cursor ? Number(input.cursor) : 0;
      const items = pool.slice(start, start + input.limit);
      return ok({ items, next_cursor: start + items.length < pool.length ? String(start + items.length) : null });
    };
    renderPage();
    const items = () => within(screen.getByRole('region', { name: '会话列表' })).getAllByRole('listitem');
    await waitFor(() => expect(items()).toHaveLength(30));
    for (let page = 2; page <= 10; page += 1) {
      fireEvent.click(await screen.findByRole('button', { name: '加载更多会话' }));
      await waitFor(() => expect(items()).toHaveLength(30 * page));
    }
    expect(calls('list_chat_sessions')[1]).toEqual({ input: { cursor: '30', limit: 30 } });
    expect(screen.queryByRole('button', { name: '加载更多会话' })).toBeNull();
    expect(screen.getByText(/已显示最近 300 个会话/)).toBeTruthy();
  });
});

describe('undecided skill', () => {
  const undecided = () => session({ no_skill_confirmed: false, skill_id: null });

  it('blocks sending until a skill or an explicit no-skill is applied, then sends', async () => {
    handlers.list_chat_sessions = () => ok({ items: [undecided()], next_cursor: null });
    handlers.update_chat_session_target = () => ok(session());
    await openSession();
    typeDraft('需要先决定技能');
    expect(screen.getByText(/这个会话还没有选择技能/)).toBeTruthy();
    expect(sendButton().disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: '去选择技能' }));
    expect(document.activeElement).toBe(screen.getByRole('combobox', { name: '技能' }));

    await choose('技能', '明确不使用技能');
    fireEvent.click(screen.getByRole('button', { name: '应用目标' }));
    await waitFor(() => expect(sendButton().disabled).toBe(false));
    expect(calls('update_chat_session_target')).toEqual([
      { input: { session_id: 's1', provider_id: 'p1', model_id: 'chat1', skill_id: null, no_skill: true } },
    ]);
    expect(screen.queryByText(/这个会话还没有选择技能/)).toBeNull();
    deferSend();
    fireEvent.click(sendButton());
    expect((await sentArgs()).input.content).toBe('需要先决定技能');
  });

  it('1006013 on a pinned skill that became unavailable offers re-choosing the skill', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session({ skill_id: 'summary', skill_version_id: 'summary@1.0.0', no_skill_confirmed: false })], next_cursor: null });
    handlers.send_chat_message = () => fail(1006013);
    await openSession();
    typeDraft('技能被禁用后发送');
    fireEvent.click(sendButton());
    expect((await screen.findByRole('alert')).textContent).toContain('会话固定的技能不可用，请重新选择技能或明确选择无技能。');
    expect(screen.getByText('会话固定的技能现在不可用。')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '重新选择技能' }));
    expect(document.activeElement).toBe(screen.getByRole('combobox', { name: '技能' }));
    expect(draftBox().value).toBe('技能被禁用后发送');
  });
});

describe('send, stream and terminal snapshot', () => {
  it('events that arrive before invoke resolves are buffered and the snapshot wins', async () => {
    await openSession();
    const resolveAck = deferSend();
    typeDraft('你好');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    expect(args.input).toEqual({ session_id: 's1', content: '你好', client_request_id: expect.stringMatching(UUID) });
    expect(args.onEvent).toBeInstanceOf(MockChannel);
    expect(args.onEvent).toBe(MockChannel.last());
    const channel = args.onEvent as MockChannel;
    emit(channel, { kind: 'started', task_id: args.input.client_request_id, assistant_message_id: 'a1' });
    emit(channel, { kind: 'delta', assistant_message_id: 'a1', text: '草稿片段' });
    emit(channel, { kind: 'done', message: message(2, 'assistant', '最终回复', { id: 'a1' }) });
    expect(screen.queryByText(/草稿片段|最终回复/)).toBeNull();

    await settle(resolveAck, ackFor(args));
    await screen.findByText('最终回复');
    expect(screen.queryByText(/草稿片段/)).toBeNull();
    expect(screen.queryByRole('article', { name: '助手消息（生成中）' })).toBeNull();
    expect(screen.getByText('你好')).toBeTruthy();
    await waitFor(() => expect(draftBox().value).toBe(''));
  });

  it('the terminal snapshot replaces the streamed text instead of extending it', async () => {
    await openSession();
    const resolveAck = deferSend();
    typeDraft('请回答');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    const channel = args.onEvent as MockChannel;
    await settle(resolveAck, ackFor(args));
    emit(channel, { kind: 'delta', assistant_message_id: 'a1', text: '累积' });
    emit(channel, { kind: 'delta', assistant_message_id: 'a1', text: '文本' });
    emit(channel, { kind: 'reasoning_delta', assistant_message_id: 'a1', text: '内部推理' });
    await screen.findByText('累积文本');
    expect(screen.getByRole('article', { name: '助手消息（生成中）' })).toBeTruthy();
    expect(screen.queryByText('内部推理')).toBeNull();

    emit(channel, { kind: 'done', message: message(2, 'assistant', '宿主保存的回复', { id: 'a1' }) });
    await screen.findByText('宿主保存的回复');
    expect(screen.queryByText(/累积文本/)).toBeNull();
    expect(screen.queryByText(/内部推理/)).toBeNull();
    expect(screen.getAllByRole('article', { name: '助手消息' })).toHaveLength(1);
  });

  it('stop calls cancel_chat_generation with { taskId } and the interrupted terminal shows its state', async () => {
    await openSession();
    const resolveAck = deferSend();
    typeDraft('写一篇长文');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    const channel = args.onEvent as MockChannel;
    const taskId = args.input.client_request_id;
    await settle(resolveAck, ackFor(args));
    emit(channel, { kind: 'delta', assistant_message_id: 'a1', text: '写到一半' });
    fireEvent.click(await screen.findByRole('button', { name: '停止生成' }));
    await waitFor(() => expect(calls('cancel_chat_generation')).toEqual([{ taskId }]));

    emit(channel, {
      kind: 'error', assistant_message_id: 'a1', error_code: 'task_cancelled', provider_code: null, result_unknown: true, regeneration_discarded: false,
      message: message(2, 'assistant', '写到一半', { id: 'a1', status: 'interrupted', error_code: 'task_cancelled', total_tokens: null, result_unknown: true }),
    });
    const article = await screen.findByRole('article', { name: '助手消息' });
    expect(within(article).getByText('已中断')).toBeTruthy();
    expect(within(article).getByText('用量未知')).toBeTruthy();
    expect(within(article).getByText('结果未知，上游可能已计费。')).toBeTruthy();
    expect(screen.getByText(/已停止生成。结果未知，上游可能已计费。/)).toBeTruthy();
  });

  it('stop before the acknowledgement cancels as soon as the host names the task', async () => {
    await openSession();
    const resolveAck = deferSend();
    typeDraft('马上停止');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    fireEvent.click(await screen.findByRole('button', { name: '停止生成' }));
    expect(calls('cancel_chat_generation')).toHaveLength(0);
    await settle(resolveAck, ackFor(args));
    await waitFor(() => expect(calls('cancel_chat_generation')).toEqual([{ taskId: args.input.client_request_id }]));
  });

  it('unmounting the page cancels the running generation', async () => {
    const view = await openSession();
    const resolveAck = deferSend();
    typeDraft('离开页面');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    view.unmount();
    await waitFor(() => expect(calls('cancel_chat_generation')).toEqual([{ taskId: args.input.client_request_id }]));
  });

  it('leaving the chat tab cancels the running generation', async () => {
    await openSession('季度复盘', 'workbench');
    const resolveAck = deferSend();
    typeDraft('切换标签');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    fireEvent.mouseDown(screen.getByRole('tab', { name: '单任务' }), { button: 0 });
    await waitFor(() => expect(calls('cancel_chat_generation')).toEqual([{ taskId: args.input.client_request_id }]));
  });

  it('a send has no tools key and no accept_destination unless they were chosen', async () => {
    await openSession();
    deferSend();
    typeDraft('纯文本');
    fireEvent.click(sendButton());
    const { input } = await sentArgs();
    expect(Object.keys(input).sort()).toEqual(['client_request_id', 'content', 'session_id']);
  });

  it('dropped turns are announced on the reply and the context disclosure is on the page', async () => {
    await openSession();
    expect(screen.getByText('上下文可能省略较早的完整轮次，被省略的轮次不会发送给模型。')).toBeTruthy();
    const resolveAck = deferSend();
    typeDraft('很长的对话');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    const channel = args.onEvent as MockChannel;
    await settle(resolveAck, ackFor(args, { dropped_turns: 2 }));
    emit(channel, { kind: 'done', message: message(2, 'assistant', '好的', { id: 'a1' }) });
    await screen.findByText('较早的完整轮次未发送给模型（已省略 2 轮）。');
  });

  it('IME composition never sends, a plain Enter does and Shift+Enter does not', async () => {
    await openSession();
    deferSend();
    typeDraft('输入法');
    const box = draftBox();
    fireEvent.keyDown(box, { key: 'Enter', isComposing: true });
    fireEvent.keyDown(box, { key: 'Enter', keyCode: 229 });
    fireEvent.compositionStart(box);
    fireEvent.keyDown(box, { key: 'Enter' });
    fireEvent.keyDown(box, { key: 'Enter', shiftKey: true });
    expect(calls('send_chat_message')).toHaveLength(0);
    fireEvent.compositionEnd(box);
    fireEvent.keyDown(box, { key: 'Enter' });
    expect((await sentArgs()).input.content).toBe('输入法');
  });
});

describe('destination_changed (1006012)', () => {
  it('asks for confirmation, keeps the draft, then resends with the fingerprint and a new request id', async () => {
    await openSession();
    let attempt = 0;
    handlers.send_chat_message = () => (++attempt === 1
      ? fail(1006012, { fingerprint: 'fp-new', host: 'api.new.example.invalid' })
      : ok({ task_id: 't2', user_message_id: 'u2', assistant_message_id: 'a2', dropped_turns: 0, replayed: false }));
    typeDraft('目标变了');
    fireEvent.click(sendButton());
    const dialog = await screen.findByRole('dialog', { name: 'Provider 目标已变化' });
    expect(dialog.textContent).toContain('api.new.example.invalid');
    // The open dialog hides the page behind it from the accessibility tree, so look the field up with hidden: true.
    expect((screen.getByRole('textbox', { name: '消息输入', hidden: true }) as HTMLTextAreaElement).value).toBe('目标变了');
    expect(screen.queryByRole('alert')).toBeNull();

    fireEvent.click(within(dialog).getByRole('button', { name: '确认并发送' }));
    const second = await sentArgs(2);
    expect(second.input).toEqual({
      session_id: 's1', content: '目标变了', client_request_id: expect.stringMatching(UUID), accept_destination: 'fp-new',
    });
    expect(second.input.client_request_id).not.toBe(calls('send_chat_message')[0].input.client_request_id);
    await waitFor(() => expect(draftBox().value).toBe(''));
  });

  it('cancelling the confirmation sends nothing more and keeps the draft', async () => {
    await openSession();
    handlers.send_chat_message = () => fail(1006012, { fingerprint: 'fp-new', host: 'api.new.example.invalid' });
    typeDraft('先不发');
    fireEvent.click(sendButton());
    const dialog = await screen.findByRole('dialog', { name: 'Provider 目标已变化' });
    fireEvent.click(within(dialog).getByRole('button', { name: '取消' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(calls('send_chat_message')).toHaveLength(1);
    expect(draftBox().value).toBe('先不发');
  });

  it('regenerate takes the same confirmation path', async () => {
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '问'), message(2, 'assistant', '答')], next_before_seq: null });
    await openSession();
    let attempt = 0;
    handlers.regenerate_chat_message = () => (++attempt === 1
      ? fail(1006012, { fingerprint: 'fp-r', host: 'api.new.example.invalid' })
      : ok({ task_id: 't3', user_message_id: 'm1', assistant_message_id: 'a3', dropped_turns: 0, replayed: false }));
    fireEvent.click(await screen.findByRole('button', { name: '重新生成' }));
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Provider 目标已变化' })).getByRole('button', { name: '确认并发送' }));
    await waitFor(() => expect(calls('regenerate_chat_message')).toHaveLength(2));
    expect(calls('regenerate_chat_message')[1].input).toEqual({
      message_id: 'm2', client_request_id: expect.stringMatching(UUID), accept_destination: 'fp-r',
    });
  });

  it('a 1006012 without a usable detail is a plain error, not a dialog', async () => {
    await openSession();
    handlers.send_chat_message = () => fail(1006012);
    typeDraft('没有指纹');
    fireEvent.click(sendButton());
    expect((await screen.findByRole('alert')).textContent).toContain('Provider 的目标地址已变化，确认后才能发送。');
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});

describe('error mapping', () => {
  it.each([
    [1006008, '上下文过长，请新建会话后再试。'],
    [1006009, '还没有可用的 Provider，请先到 Provider 页面完成配置。'],
    [1006010, '已有生成正在进行，请先停止当前生成。'],
    [1006011, '请求标识冲突，请重新发送。'],
    [1006013, '会话固定的技能不可用，请重新选择技能或明确选择无技能。'],
    [1006015, '请先为会话选择 Provider 和模型。'],
    [1006017, '工具确认已失效、已过期或与当前提案不符，请重新发送并重新选择工具。'],
  ])('send rejected with %i shows fixed Chinese text, keeps the draft and never the host message', async (code, text) => {
    await openSession();
    handlers.send_chat_message = () => fail(code);
    typeDraft('会被拒绝');
    fireEvent.click(sendButton());
    expect((await screen.findByRole('alert')).textContent).toContain(text);
    expect(screen.queryByText(/host text must never be shown/)).toBeNull();
    expect(draftBox().value).toBe('会被拒绝');
    expect(sendButton().disabled).toBe(false);
  });

  it('1006001 on send marks the session as gone and blocks further sends', async () => {
    await openSession();
    handlers.send_chat_message = () => fail(1006001);
    typeDraft('会话已被删除');
    fireEvent.click(sendButton());
    await screen.findByText('这个会话已不存在，无法继续发送。');
    expect(draftBox().disabled).toBe(true);
  });

  it('1006014 is shown when the last-assistant rule is violated and no command is sent', async () => {
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '问一'), message(2, 'assistant', '答一'), message(3, 'user', '问二')], next_before_seq: null });
    await openSession();
    await screen.findByText('问二');
    await act(async () => { await useChatStore.getState().regenerate('m2'); });
    expect((await screen.findByRole('alert')).textContent).toContain('只能重新生成会话中最后一条助手回复。');
    expect(calls('regenerate_chat_message')).toHaveLength(0);
  });

  it('a failed terminal event shows the failed badge, the category text and the billing warning', async () => {
    await openSession();
    const resolveAck = deferSend();
    typeDraft('会超时');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    emit(args.onEvent as MockChannel, {
      kind: 'error', assistant_message_id: 'a1', error_code: 'provider_timeout', provider_code: null, result_unknown: true, regeneration_discarded: false,
      message: message(2, 'assistant', '', { id: 'a1', status: 'failed', error_code: 'provider_timeout', total_tokens: null, result_unknown: true }),
    });
    const article = await screen.findByRole('article', { name: '助手消息' });
    expect(within(article).getByText('生成失败')).toBeTruthy();
    expect(within(article).getByText('请求超时。')).toBeTruthy();
    expect(within(article).getByText('结果未知，上游可能已计费。')).toBeTruthy();
    expect(screen.getByText('请求超时。结果未知，上游可能已计费。')).toBeTruthy();
  });
});

describe('persisted message states', () => {
  it('shows 用量未知 for null usage (never 0) and the real number when known', async () => {
    handlers.list_chat_messages = () => ok({
      items: [
        message(1, 'user', '问'),
        message(2, 'assistant', '未知用量', { total_tokens: null }),
        message(3, 'user', '再问'),
        message(4, 'assistant', '已知用量', { total_tokens: 42 }),
      ],
      next_before_seq: null,
    });
    await openSession();
    const [unknown, known] = await screen.findAllByRole('article', { name: '助手消息' });
    expect(within(unknown).getByText('用量未知')).toBeTruthy();
    expect(within(known).getByText('用量 42 tokens')).toBeTruthy();
    expect(screen.queryByText(/用量 0/)).toBeNull();
  });

  it('result_unknown messages say the upstream may have billed', async () => {
    handlers.list_chat_messages = () => ok({
      items: [message(1, 'user', '问'), message(2, 'assistant', '半截', { status: 'interrupted', error_code: 'process_interrupted', total_tokens: null, result_unknown: true })],
      next_before_seq: null,
    });
    await openSession();
    const article = await screen.findByRole('article', { name: '助手消息' });
    expect(within(article).getByText('结果未知，上游可能已计费。')).toBeTruthy();
    expect(within(article).getByText('已中断')).toBeTruthy();
    expect(within(article).getByText('生成因应用退出或重启而中断。')).toBeTruthy();
  });

  it('failed messages carry the failed badge and the category text', async () => {
    handlers.list_chat_messages = () => ok({
      items: [message(1, 'user', '问'), message(2, 'assistant', '', { status: 'failed', error_code: 'empty_response', total_tokens: null })],
      next_before_seq: null,
    });
    await openSession();
    const article = await screen.findByRole('article', { name: '助手消息' });
    expect(within(article).getByText('生成失败')).toBeTruthy();
    expect(within(article).getByText('模型返回了空回复。')).toBeTruthy();
  });

  it('a reply cut at the length limit says so, and only that reply', async () => {
    // D075/D080: `finish_reason = length` is a completed reply the user must be told is truncated.
    handlers.list_chat_messages = () => ok({
      items: [
        message(1, 'user', '问'),
        message(2, 'assistant', '被截断的回答', { finish_reason: 'length' }),
        message(3, 'user', '再问'),
        message(4, 'assistant', '完整的回答', { finish_reason: 'stop' }),
      ],
      next_before_seq: null,
    });
    await openSession();
    const [cut, whole] = await screen.findAllByRole('article', { name: '助手消息' });
    expect(within(cut).getByText('回复因达到长度上限被截断。')).toBeTruthy();
    expect(within(whole).queryByText('回复因达到长度上限被截断。')).toBeNull();
  });
});

describe('regenerate', () => {
  const history = () => ok({
    items: [message(1, 'user', '问一'), message(2, 'assistant', '答一'), message(3, 'user', '问二'), message(4, 'assistant', '答二')],
    next_before_seq: null,
  });

  it('is offered only on the last assistant message', async () => {
    handlers.list_chat_messages = history;
    await openSession();
    const [first, last] = await screen.findAllByRole('article', { name: '助手消息' });
    expect(within(first).queryByRole('button', { name: '重新生成' })).toBeNull();
    expect(within(last).getByRole('button', { name: '重新生成' })).toBeTruthy();
    expect(screen.getAllByRole('button', { name: '重新生成' })).toHaveLength(1);
  });

  it('is not offered when the last message is the user message', async () => {
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '问一'), message(2, 'assistant', '答一'), message(3, 'user', '问二')], next_before_seq: null });
    await openSession();
    await screen.findByText('问二');
    expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull();
  });

  it('sends { input: { message_id, client_request_id }, onEvent } and swaps the reply for the new snapshot', async () => {
    handlers.list_chat_messages = history;
    await openSession();
    const { promise, resolve } = Promise.withResolvers<unknown>();
    handlers.regenerate_chat_message = () => promise;
    fireEvent.click(await screen.findByRole('button', { name: '重新生成' }));
    await waitFor(() => expect(calls('regenerate_chat_message')).toHaveLength(1));
    const args = calls('regenerate_chat_message')[0];
    expect(args).toEqual({ input: { message_id: 'm4', client_request_id: expect.stringMatching(UUID) }, onEvent: expect.any(MockChannel) });
    expect(Object.keys(args).sort()).toEqual(['input', 'onEvent']);
    expect(screen.queryByText('答二')).toBeNull();
    expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull();

    await act(async () => { resolve(ok({ task_id: args.input.client_request_id, user_message_id: 'm3', assistant_message_id: 'a5', dropped_turns: 0, replayed: false })); });
    emit(args.onEvent, { kind: 'done', message: message(4, 'assistant', '新的答二', { id: 'a5' }) });
    await screen.findByText('新的答二');
    expect(screen.queryByText('答二')).toBeNull();
    expect(screen.getAllByRole('article', { name: '助手消息' })).toHaveLength(2);
    expect(screen.getAllByRole('button', { name: '重新生成' })).toHaveLength(1);
  });

  it('a discarded regeneration restores the kept reply and says so', async () => {
    handlers.list_chat_messages = history;
    await openSession();
    const { promise, resolve } = Promise.withResolvers<unknown>();
    handlers.regenerate_chat_message = () => promise;
    fireEvent.click(await screen.findByRole('button', { name: '重新生成' }));
    await waitFor(() => expect(calls('regenerate_chat_message')).toHaveLength(1));
    const args = calls('regenerate_chat_message')[0];
    await act(async () => { resolve(ok({ task_id: args.input.client_request_id, user_message_id: 'm3', assistant_message_id: 'a5', dropped_turns: 0, replayed: false })); });
    emit(args.onEvent, {
      kind: 'error', assistant_message_id: 'a5', error_code: 'task_cancelled', provider_code: null, result_unknown: false, regeneration_discarded: true,
      message: message(4, 'assistant', '答二'),
    });
    await screen.findByText('答二');
    expect(screen.getByText(/重新生成未完成，已保留原回复。已停止生成。/)).toBeTruthy();
    expect(screen.getAllByRole('article', { name: '助手消息' })).toHaveLength(2);
  });
});

describe('keyset paging windows', () => {
  it('older pages are loaded by before_seq, the window stops at 200 and 回到最新 re-anchors', async () => {
    const all = Array.from({ length: 300 }, (_, i) => message(i + 1, 'user', `消息 ${i + 1}`));
    handlers.list_chat_messages = ({ input }) => {
      const before = input.before_seq ?? 301;
      const page = all.filter(item => item.seq < before).slice(-input.limit);
      const first = page[0]?.seq ?? 1;
      return ok({ items: page, next_before_seq: first > 1 ? first : null });
    };
    await openSession();
    const items = () => within(screen.getByRole('list', { name: '消息' })).getAllByRole('listitem');
    await waitFor(() => expect(items()).toHaveLength(50));
    expect(calls('list_chat_messages')[0]).toEqual({ input: { session_id: 's1', limit: 50 } });

    for (const size of [100, 150, 200]) {
      fireEvent.click(await screen.findByRole('button', { name: '加载更早的消息' }));
      await waitFor(() => expect(items()).toHaveLength(size));
    }
    expect(calls('list_chat_messages').map(args => args.input.before_seq)).toEqual([undefined, 251, 201, 151]);
    expect(screen.queryByRole('button', { name: '回到最新' })).toBeNull();

    fireEvent.click(await screen.findByRole('button', { name: '加载更早的消息' }));
    await screen.findByRole('button', { name: '回到最新' });
    expect(items()).toHaveLength(200);
    expect(screen.getByText('为控制页面规模，较新的消息已从当前窗口移除。')).toBeTruthy();
    expect(screen.getByText('消息 51')).toBeTruthy();
    expect(screen.queryByText('消息 300')).toBeNull();

    fireEvent.click(screen.getByRole('button', { name: '回到最新' }));
    await screen.findByText('消息 300');
    await waitFor(() => expect(items()).toHaveLength(50));
    expect(screen.queryByRole('button', { name: '回到最新' })).toBeNull();
    expect(calls('list_chat_messages').pop()).toEqual({ input: { session_id: 's1', limit: 50 } });
  });
});

describe('copy feedback', () => {
  const copyHistory = () => {
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '问'), message(2, 'assistant', '可复制的回答')], next_before_seq: null });
  };

  it('announces success after the clipboard write resolves', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    copyHistory();
    await openSession();
    const article = await screen.findByRole('article', { name: '助手消息' });
    fireEvent.click(within(article).getByRole('button', { name: '复制' }));
    await screen.findByText('已复制');
    expect(writeText).toHaveBeenCalledWith('可复制的回答');
  });

  it('shows a visible failure when both clipboard paths fail', async () => {
    Object.defineProperty(navigator, 'clipboard', { value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) }, configurable: true });
    Object.defineProperty(document, 'execCommand', { value: vi.fn().mockReturnValue(false), configurable: true });
    copyHistory();
    await openSession();
    const article = await screen.findByRole('article', { name: '助手消息' });
    fireEvent.click(within(article).getByRole('button', { name: '复制' }));
    await screen.findByText('复制失败，请手动选择文本复制');
    expect(screen.queryByText('已复制')).toBeNull();
  });
});

describe('MCP tool turn', () => {
  const startToolTurn = async () => {
    await openSession();
    await pickTools(/read_file/);
    typeDraft('读取合成文件');
    const resolveAck = deferSend();
    fireEvent.click(sendButton());
    const args = await sentArgs();
    const channel = args.onEvent as MockChannel;
    await settle(resolveAck, ackFor(args));
    emit(channel, { kind: 'started', task_id: args.input.client_request_id, assistant_message_id: 'a1' });
    emit(channel, { kind: 'tool_proposal', assistant_message_id: 'a1', proposal: proposal() });
    return { args, channel, taskId: args.input.client_request_id as string };
  };

  it('shows the per-send note before any tool is opened and lists connected tools on demand', async () => {
    await openSession();
    expect(screen.getByText('工具往返不会进入后续对话上下文')).toBeTruthy();
    expect(calls('list_mcp_servers')).toHaveLength(0);
    await pickTools(/read_file/);
    expect(calls('list_mcp_tools')).toEqual([{ serverId: 'srv1' }]);
    expect(pickerLabel(1)).toBeTruthy();
  });

  it('selects at most 8 tools', async () => {
    handlers.list_mcp_tools = () => Array.from({ length: 9 }, (_, i) => tool(`tool_${i}`));
    await openSession();
    await pickTools(...Array.from({ length: 8 }, (_, i) => new RegExp(`tool_${i}\\b`)));
    expect(pickerLabel(8)).toBeTruthy();
    expect((screen.getByRole('checkbox', { name: /tool_8\b/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('a send carries the chosen tools, the proposal shows the exact arguments, and the tool choice is spent', async () => {
    const { args } = await startToolTurn();
    expect(args.input.tools).toEqual([{ server_id: 'srv1', tool_name: 'read_file', schema_hash: 'hash-read_file' }]);
    expect(screen.getByRole('heading', { name: '确认工具调用' })).toBeTruthy();
    expect(screen.getByText('文件服务 / read_file', { selector: 'strong' })).toBeTruthy();
    expect(screen.getByLabelText('工具调用参数').textContent).toBe(JSON.stringify({ path: '/synthetic/a.txt', limit: 3 }, null, 2));
    expect(screen.getByText(/剩余 \d+ 秒/)).toBeTruthy();
    expect(pickerLabel(0)).toBeTruthy();
    expect(screen.getByText('等待你确认工具调用…')).toBeTruthy();
  });

  it.each([
    ['approve', /批准本次调用/],
    ['deny', /拒绝并停止/],
  ] as const)('%s calls advance_chat_tool_turn on the original channel only and later sends do not re-send tools', async (type, name) => {
    const { channel, taskId } = await startToolTurn();
    handlers.advance_chat_tool_turn = () => ok(true);
    fireEvent.click(screen.getByRole('button', { name }));
    await waitFor(() => expect(calls('advance_chat_tool_turn')).toHaveLength(1));
    expect(calls('advance_chat_tool_turn')[0]).toEqual({ input: { task_id: taskId, action: { type, proposal_id: 'prop1' } } });
    expect(Object.keys(calls('advance_chat_tool_turn')[0])).toEqual(['input']);
    await waitFor(() => expect(screen.queryByRole('heading', { name: '确认工具调用' })).toBeNull());
    expect(screen.getByText('正在执行工具并等待模型…')).toBeTruthy();

    emit(channel, { kind: 'done', message: message(2, 'assistant', '工具结果已整理', { id: 'a1' }) });
    await screen.findByText('工具结果已整理');
    deferSend();
    typeDraft('谢谢');
    fireEvent.click(sendButton());
    const next = await sentArgs(2);
    expect(next.input.content).toBe('谢谢');
    expect(next.input).not.toHaveProperty('tools');
  });

  it('an expired proposal cannot be approved or denied', async () => {
    await openSession();
    await pickTools(/read_file/);
    typeDraft('读取');
    const resolveAck = deferSend();
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    emit(args.onEvent as MockChannel, { kind: 'tool_proposal', assistant_message_id: 'a1', proposal: proposal({ expires_at: Math.floor(Date.now() / 1000) - 5 }) });
    await screen.findByText('提案已过期，无法再批准。');
    expect((screen.getByRole('button', { name: /批准本次调用/ }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: /拒绝并停止/ }) as HTMLButtonElement).disabled).toBe(true);
    expect(calls('advance_chat_tool_turn')).toHaveLength(0);
  });

  it('a stale decision (1006017) removes the proposal and explains how to continue', async () => {
    await startToolTurn();
    handlers.advance_chat_tool_turn = () => fail(1006017);
    fireEvent.click(screen.getByRole('button', { name: /批准本次调用/ }));
    expect((await screen.findByRole('alert')).textContent).toContain('工具确认已失效、已过期或与当前提案不符，请重新发送并重新选择工具。');
    expect(screen.queryByRole('heading', { name: '确认工具调用' })).toBeNull();
    expect(screen.getByText(/请先点击“停止生成”结束这一轮/)).toBeTruthy();
    expect(screen.getByRole('button', { name: '停止生成' })).toBeTruthy();
  });

  it('1006017 on send clears the tool selection and reloads the tool list on the next open', async () => {
    await openSession();
    await pickTools(/read_file/);
    expect(pickerLabel(1)).toBeTruthy();
    handlers.send_chat_message = () => fail(1006017);
    typeDraft('工具已变化');
    fireEvent.click(sendButton());
    expect((await screen.findByRole('alert')).textContent).toContain('工具确认已失效');
    await screen.findByText('工具选择已清空。请在下方重新选择工具后再发送。');
    expect(pickerLabel(0)).toBeTruthy();
    expect(draftBox().value).toBe('工具已变化');
    expect(calls('list_mcp_servers')).toHaveLength(1);
    await pickTools(/read_file/);
    expect(calls('list_mcp_servers')).toHaveLength(2);
    expect(pickerLabel(1)).toBeTruthy();
  });
});

describe('disclosures and deletion', () => {
  const DISCLOSURES = [
    '聊天正文保存在本机数据库。',
    '删除会话后应用内不可恢复，不承诺物理擦除。',
    '任务审计记录会保留，且不含正文。',
    '应用被强制结束时，本次未提交的生成内容可能丢失。',
    '上下文可能省略较早的完整轮次，被省略的轮次不会发送给模型。',
  ];

  it('shows every mandatory disclosure with and without an open session', async () => {
    renderPage();
    await screen.findByRole('button', { name: /季度复盘/ });
    for (const text of DISCLOSURES) expect(screen.getByText(text)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /季度复盘/ }));
    await screen.findByRole('textbox', { name: '消息输入' });
    for (const text of DISCLOSURES) expect(screen.getByText(text)).toBeTruthy();
  });

  it('deleting asks first, states the limits and then deletes by session_id', async () => {
    handlers.delete_chat_session = () => ok(true);
    await openSession();
    fireEvent.click(screen.getByRole('button', { name: '删除会话' }));
    const dialog = await screen.findByRole('dialog', { name: '删除会话' });
    expect(dialog.textContent).toContain('确定删除？应用内将无法恢复。已删除内容可能残留在数据库空闲页和备份中。');
    expect(calls('delete_chat_session')).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole('button', { name: '删除' }));
    await waitFor(() => expect(calls('delete_chat_session')).toEqual([{ input: { session_id: 's1' } }]));
    await screen.findByRole('heading', { name: '开始一次对话' });
  });
});

describe('identity', () => {
  it('no command of a full create, send, stop, regenerate and tool flow carries a user id', async () => {
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '问'), message(2, 'assistant', '答')], next_before_seq: null });
    handlers.advance_chat_tool_turn = () => ok(true);
    await openSession();
    await pickTools(/read_file/);
    typeDraft('带工具');
    const resolveAck = deferSend();
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    emit(args.onEvent as MockChannel, { kind: 'tool_proposal', assistant_message_id: 'a1', proposal: proposal() });
    fireEvent.click(await screen.findByRole('button', { name: /批准本次调用/ }));
    await waitFor(() => expect(calls('advance_chat_tool_turn')).toHaveLength(1));
    fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
    await waitFor(() => expect(calls('cancel_chat_generation')).toHaveLength(1));
    const commands = new Set(mocked.mock.calls.map(([name]) => name));
    for (const name of ['list_chat_sessions', 'list_chat_messages', 'send_chat_message', 'advance_chat_tool_turn', 'cancel_chat_generation']) {
      expect(commands.has(name)).toBe(true);
    }
    expect(keysOf(mocked.mock.calls.map(([, payload]) => payload)).some(key => /^user_?id$/i.test(key))).toBe(false);
  });
});

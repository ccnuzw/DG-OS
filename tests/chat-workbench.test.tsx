/* eslint-disable @typescript-eslint/no-explicit-any */
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter, useLocation, useNavigate } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import Workbench from '../src/components/chat/ChatWorkbench';
import ChatWorkbench from '../src/components/chat/ChatWorkbench';
import { useChatStore } from '../src/stores/chatStore';
import { MockChannel } from './helpers/mockChannel';
import { useWorkspaceTabs } from '../src/workspace/useWorkspaceTabs';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { setLocaleForTesting } from '../src/i18n';

import { chatBudget } from './helpers/chatBudget';
import { defaultAiFoundation } from '../src/types/ai';
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
  id: 's1', title: '季度复盘', title_source: 'user', provider_id: 'p1', model_id: 'chat1', skills: [],
  generation: {}, no_skill_confirmed: true, created_at: 1, updated_at: 2, ...over,
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
    inspect_chat_context: () => ok(chatBudget()),
    list_chat_runs: () => ok({ items: [] }),
    chat_skill_catalog: () => ok({ items: [], recommendations: [] }),
    cancel_chat_generation: () => ok(true),
    list_providers: () => ok([{ id: 'p1', name: '合成服务', enabled: true, credential_status: 'stored' }]),
    list_models: () => ok([{ id: 'mm1', model_id: 'chat1', provider_id: 'p1', model_name: '对话模型', model_type: 'text-generation' }]),
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
  return render(<MemoryRouter  future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>{page === 'chat' ? <ChatWorkbench /> : <Workbench />}</MemoryRouter>);
}

async function openSession(title = '季度复盘', page: 'chat' | 'workbench' = 'chat') {
  const view = renderPage(page);
  fireEvent.click(await screen.findByRole('tab', { name: new RegExp(title) }));
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
  if (name === 'Provider' || name === '模型') {
    if (!screen.queryByRole('dialog', { name: '选择 Provider 和模型' })) fireEvent.click(screen.getByRole('button', { name: '会话目标' }));
    fireEvent.click(await within(screen.getByRole('dialog', { name: '选择 Provider 和模型' })).findByRole('button', { name: option, exact: true }));
    return;
  }
  const trigger = await screen.findByRole('combobox', { name });
  await waitFor(() => expect((trigger as HTMLButtonElement).disabled).toBe(false));
  trigger.focus();
  fireEvent.keyDown(trigger, { key: 'Enter' });
  fireEvent.click(within(await screen.findByRole('listbox')).getByRole('option', { name: option }));
  await waitFor(() => expect(screen.queryByRole('listbox')).toBeNull());
}

async function openMorePage(page: 'tools' | 'voice') {
  if (!screen.queryByRole('dialog', { name: '更多功能' })) fireEvent.click(screen.getByRole('button', { name: '更多功能' }));
  const panel = await screen.findByRole('dialog', { name: '更多功能' });
  const navigation = within(panel).getByRole('navigation', { name: '更多功能' });
  const action = within(navigation).getByRole('button', { name: page === 'tools' ? /MCP 工具（可选/ : '语音输入' });
  if (action.getAttribute('aria-pressed') !== 'true') fireEvent.click(action);
  return panel;
}

async function closePanel() {
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
}

async function pickTools(...names: RegExp[]) {
  const panel = await openMorePage('tools');
  for (const name of names) fireEvent.click(await within(panel).findByRole('checkbox', { name }));
}

async function openRetainedRun() {
  fireEvent.click(screen.getByRole('button', { name: '执行历史' }));
  const history = await screen.findByRole('dialog', { name: '执行历史' });
  fireEvent.click(await within(within(history).getByRole('group', { name: '执行历史' })).findByRole('button', { name: /季度复盘/ }));
}

describe('sessions', () => {
  it.each(['Escape', 'cancel'])('returns focus to the session tab after %s without losing the conversation draft', async method => {
    await openSession();
    typeDraft('保留消息草稿');
    const tab = screen.getByRole('tab', { name: /季度复盘/ });
    fireEvent.contextMenu(tab);
    fireEvent.click(await screen.findByRole('menuitem', { name: '重命名会话' }));
    const title = await screen.findByRole('textbox', { name: '会话标题' });
    fireEvent.change(title, { target: { value: '未保存标题' } });
    if (method === 'Escape') fireEvent.keyDown(title, { key: 'Escape' });
    else fireEvent.click(screen.getByRole('button', { name: '取消' }));
    await waitFor(() => expect(document.activeElement).toBe(tab));
    expect(draftBox().value).toBe('保留消息草稿');
    expect(calls('rename_chat_session')).toEqual([]);
  });

  it('renames a background tab without switching sessions or losing the active draft, and keeps failed edits open', async () => {
    const background = session({ id: 's2', title: '后台会话' });
    handlers.list_chat_sessions = () => ok({ items: [session(), background], next_cursor: null });
    await openSession();
    await act(async () => { await useChatStore.getState().selectSession(background); await useChatStore.getState().selectSession(session()); });
    typeDraft('保留活动草稿');
    fireEvent.contextMenu(screen.getByRole('tab', { name: '后台会话' }));
    fireEvent.click(await screen.findByRole('menuitem', { name: '重命名会话' }));
    const title = await screen.findByRole('textbox', { name: '会话标题' });
    handlers.rename_chat_session = () => fail(1006007);
    fireEvent.change(title, { target: { value: '新的后台标题' } });
    fireEvent.click(screen.getByRole('button', { name: '保存标题' }));
    await screen.findByRole('alert');
    expect((title as HTMLInputElement).value).toBe('新的后台标题');
    handlers.rename_chat_session = ({ input }) => ok({ ...background, title: input.title });
    fireEvent.click(screen.getByRole('button', { name: '保存标题' }));
    await screen.findByRole('tab', { name: '新的后台标题' });
    expect(useChatStore.getState().activeSessionId).toBe('s1');
    expect(draftBox().value).toBe('保留活动草稿');
  });

  it('opens the tab menu with the keyboard and closes only the targeted background tab', async () => {
    const background = session({ id: 's2', title: '后台会话' });
    handlers.list_chat_sessions = () => ok({ items: [session(), background], next_cursor: null });
    await openSession();
    await act(async () => { await useChatStore.getState().selectSession(background); await useChatStore.getState().selectSession(session()); });
    const tab = screen.getByRole('tab', { name: '后台会话' });
    tab.focus();
    fireEvent.keyDown(tab, { key: 'F10', shiftKey: true });
    fireEvent.click(await screen.findByRole('menuitem', { name: '关闭会话' }));
    await waitFor(() => expect(screen.queryByRole('tab', { name: '后台会话' })).toBeNull());
    expect(useChatStore.getState().activeSessionId).toBe('s1');
    expect(useChatStore.getState().sessions.some(item => item.id === 's2')).toBe(true);
    expect(calls('delete_chat_session')).toHaveLength(0);
  });

  it('creating a session from the empty state opens it with the undecided target and skill explained', async () => {
    handlers.list_chat_sessions = () => ok({ items: [], next_cursor: null });
    handlers.create_chat_session = () => ok(session({ id: 'new1', title: '新会话', title_source: 'auto', provider_id: null, model_id: null, no_skill_confirmed: false }));
    renderPage();
    await screen.findByRole('heading', { name: '开始一次对话' });
    fireEvent.click(screen.getAllByRole('button', { name: '新建会话' })[0]);
    await screen.findByRole('textbox', { name: '消息输入' });
    expect(calls('create_chat_session')).toEqual([{ input: {} }]);
    expect(screen.getByRole('tab', { name: '新会话' })).toBeTruthy();
    typeDraft('先打个招呼');
    expect(sendButton().disabled).toBe(true);
    expect(calls('send_chat_message')).toHaveLength(0);
  });

  it('selecting another session loads its newest page of messages', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session({ id: 'sA', title: '会话 A' }), session({ id: 'sB', title: '会话 B' })], next_cursor: null });
    handlers.list_chat_messages = ({ input }) => ok({ items: [message(1, 'user', `属于 ${input.session_id}`, { session_id: input.session_id })], next_before_seq: null });
    renderPage();
    fireEvent.click(screen.getByRole('button', { name: '会话历史' }));
    fireEvent.click(await screen.findByRole('button', { name: /会话 B/ }));
    await screen.findByText('属于 sB');
    expect(calls('list_chat_messages').at(-1)).toEqual({ input: { session_id: 'sB', limit: 50 } });
    expect(screen.getByRole('tab', { name: '会话 B' }).getAttribute('aria-selected')).toBe('true');
  });
  it('isolates drafts and unsaved capability targets between open session tabs', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session({ id: 'sA', title: '会话 A' }), session({ id: 'sB', title: '会话 B', provider_id: 'p2', model_id: 'modelB', skills: [{ skill_id: 'writing', version_id: 'writing@1', content_hash: 'hash' }] })], next_cursor: null });
    handlers.list_providers = () => ok([{ id: 'p1', name: '服务 A', enabled: true, credential_status: 'stored' }, { id: 'p2', name: '服务 B', enabled: true, credential_status: 'stored' }]);
    handlers.list_models = ({ providerId }) => ok([{ id: 'm', model_id: providerId === 'p2' ? 'modelB' : 'chat1', provider_id: providerId, model_name: providerId === 'p2' ? '模型 B' : 'Chat', model_type: 'text-generation' }]);
    handlers.update_chat_session_target = ({ input }) => ok(session({ id: input.session_id, title: '会话 A', provider_id: input.provider_id, model_id: input.model_id }));
    renderPage();
    await screen.findByRole('tab', { name: '会话 A' });
    typeDraft('draft A');
    await choose('Provider', '服务 B');
    await choose('模型', '模型 B');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: '回答偏好' }));
    fireEvent.change(screen.getByRole('textbox', { name: '自定义回答要求' }), { target: { value: 'draft guidance A' } });
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    fireEvent.click(screen.getByRole('button', { name: '会话历史' }));
    fireEvent.click(await screen.findByRole('button', { name: '会话 B' }));
    await screen.findByRole('textbox', { name: '消息输入' });
    typeDraft('draft B');
    expect(useChatStore.getState().sessionTargetDrafts.sA).toEqual({ provider_id: 'p2', model_id: 'modelB', skill_ids: [], no_skill: true, generation: { instruction: 'draft guidance A' } });
    expect(useChatStore.getState().sessionTargetDrafts.sB).toBeUndefined();
    fireEvent.click(screen.getByRole('tab', { name: /会话 A/ }));
    expect(draftBox().value).toBe('draft A');
    expect(useChatStore.getState().sessionTargetDrafts.sA?.provider_id).toBe('p2');
    fireEvent.click(screen.getByRole('tab', { name: /会话 B/ }));
    expect(draftBox().value).toBe('draft B');
    expect(useChatStore.getState().sessionTargetDrafts.sB).toBeUndefined();
  });
  it('switching away from a running session preserves its generation and returns to it', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session({ id: 'sA', title: '会话 A' }), session({ id: 'sB', title: '会话 B' })], next_cursor: null });
    const view = await openSession('会话 A');
    const resolveAck = deferSend();
    typeDraft('running draft');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    fireEvent.click(screen.getByRole('button', { name: '新建会话' }));
    expect(calls('cancel_chat_generation')).toHaveLength(0);
    fireEvent.click(screen.getByRole('tab', { name: /会话 A/ }));
    expect(screen.getByRole('article', { name: '助手消息（生成中）' })).toBeTruthy();
    expect(calls('cancel_chat_generation')).toHaveLength(0);
    view.unmount();
  });

  it('closing a running session requires confirmation and preserves accepted input content', async () => {
    await openSession();
    const resolveAck = deferSend();
    typeDraft('keep this draft');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    fireEvent.contextMenu(screen.getByRole('tab', { name: /季度复盘/ }));
    fireEvent.click(await screen.findByRole('menuitem', { name: '关闭会话' }));
    const dialog = await screen.findByRole('dialog', { name: '关闭会话' });
    expect(calls('cancel_chat_generation')).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole('button', { name: '关闭会话' }));
    await waitFor(() => expect(calls('cancel_chat_generation')).toEqual([{ taskId: args.input.client_request_id }]));
    expect(screen.getByRole('tab', { name: /季度复盘/, hidden: true })).toBeTruthy();
    emit(args.onEvent, { kind: 'done', message: message(2, 'assistant', '已保存的片段', { id: 'a1', status: 'interrupted' }) });
    await waitFor(() => expect(screen.queryByRole('tab', { name: /季度复盘/, hidden: true })).toBeNull());
    await act(async () => { await useChatStore.getState().selectSession(session()); });
    expect(useChatStore.getState().sessionDrafts.s1).toBe('');
    expect(useChatStore.getState().sessionViews.s1?.messages[0]?.content).toBe('keep this draft');
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
    fireEvent.click(screen.getByRole('button', { name: '会话历史' }));
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

describe('separate capability pickers', () => {
  it('saves model and ordered skill choices immediately while leaving generation settings explicit', async () => {
    handlers.list_installed_skills = () => ['summary', 'writing'].map(id => ({ skill_id: id, name: id, version: '1', enabled: true, call_type: 'content', manifest: { category: 'writing' } }));
    handlers.list_models = () => ok([{ id: 'm1', model_id: 'chat1', model_name: '对话模型', model_type: 'text-generation' }, { id: 'm2', model_id: 'chat2', model_name: '第二模型', model_type: 'text-generation' }]);
    handlers.update_chat_session_target = ({ input }) => ok(session({ provider_id: input.provider_id, model_id: input.model_id, skills: input.skill_ids.map((id: string) => ({ skill_id: id, version_id: `${id}@1`, content_hash: 'fixture' })), no_skill_confirmed: input.no_skill, generation: input.generation }));
    await openSession();
    await choose('模型', '第二模型');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: '会话目标' })));
    fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
    fireEvent.click(await within(screen.getByRole('dialog')).findByRole('button', { name: 'summary', exact: true }));
    await waitFor(() => expect(useChatStore.getState().activeSession?.skills.map(skill => skill.skill_id)).toEqual(['summary']));
    fireEvent.click(await within(screen.getByRole('dialog')).findByRole('button', { name: 'writing', exact: true }));
    await waitFor(() => expect(useChatStore.getState().activeSession?.skills.map(skill => skill.skill_id)).toEqual(['summary', 'writing']));
    fireEvent.click(screen.getByRole('button', { name: '上移 writing' }));
    await waitFor(() => expect(useChatStore.getState().activeSession?.skills.map(skill => skill.skill_id)).toEqual(['writing', 'summary']));
    await closePanel();
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: '选择 Skills' })));
    fireEvent.click(screen.getByRole('button', { name: '回答偏好' }));
    fireEvent.change(screen.getByRole('textbox', { name: '自定义回答要求' }), { target: { value: 'Preserve sources' } });
    expect(calls('update_chat_session_target')).toHaveLength(4);
    expect(useChatStore.getState().activeSession?.model_id).toBe('chat2');
    expect(useChatStore.getState().activeSession?.generation.instruction).toBeUndefined();
    fireEvent.click(screen.getByRole('button', { name: '保存回答偏好' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(useChatStore.getState().activeSession?.skills.map(skill => skill.skill_id)).toEqual(['writing', 'summary']);
    expect(useChatStore.getState().activeSession?.generation.instruction).toBe('Preserve sources');
  });
  it('keeps the current model and draft on save failure, then closes only after a successful retry', async () => {
    handlers.list_models = () => ok([{ id: 'm2', model_id: 'chat2', model_name: '第二模型', model_type: 'text-generation' }]);
    const pending = Promise.withResolvers<unknown>();
    handlers.update_chat_session_target = () => pending.promise;
    await openSession();
    typeDraft('保留消息草稿');
    act(() => useChatStore.getState().setSessionTargetDraft('s1', { provider_id: 'p1', model_id: 'chat1', skill_ids: [], no_skill: true, generation: { instruction: 'Unapplied guidance' } }));
    await choose('模型', '第二模型');
    expect(screen.getByRole('dialog', { name: '选择 Provider 和模型' })).toBeTruthy();
    expect(useChatStore.getState().activeSession?.model_id).toBe('chat1');
    await act(async () => pending.resolve(fail(1006015)));
    await screen.findByRole('alert');
    expect(draftBox().value).toBe('保留消息草稿');
    expect(useChatStore.getState().activeSession?.model_id).toBe('chat1');
    handlers.update_chat_session_target = ({ input }) => ok(session({ model_id: input.model_id, generation: input.generation }));
    await choose('模型', '第二模型');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(useChatStore.getState().activeSession?.model_id).toBe('chat2');
    expect(useChatStore.getState().activeSession?.generation).toEqual({});
    expect(useChatStore.getState().sessionTargetDrafts.s1?.generation.instruction).toBe('Unapplied guidance');
    expect(useChatStore.getState().sessionTargetDrafts.s1?.model_id).toBe('chat2');
    expect(draftBox().value).toBe('保留消息草稿');
  });
  it('preserves settings after a rejected save and allows a successful retry', async () => {
    handlers.update_chat_session_target = () => fail(1006015);
    await openSession();
    fireEvent.click(screen.getByRole('button', { name: '回答偏好' }));
    fireEvent.change(screen.getByRole('textbox', { name: '自定义回答要求' }), { target: { value: 'Needs retry' } });
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: '保存回答偏好' }));
    await within(screen.getByRole('dialog')).findByRole('alert');
    expect((screen.getByRole('textbox', { name: '自定义回答要求' }) as HTMLTextAreaElement).value).toBe('Needs retry');
    expect(useChatStore.getState().activeSession?.generation.instruction).toBeUndefined();
    handlers.update_chat_session_target = ({ input }) => ok(session({ generation: input.generation }));
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: '保存回答偏好' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(useChatStore.getState().activeSession?.generation.instruction).toBe('Needs retry');
  });
  it('saves reasoning immediately without applying other drafts, and retains the picker on failure', async () => {
    const initial = { temperature: 0.4, reasoning: { effort: 'low', budget_tokens: 2048 } };
    handlers.list_chat_sessions = () => ok({ items: [session({ generation: initial })], next_cursor: null });
    handlers.update_chat_session_target = () => fail(1006015);
    await openSession();
    typeDraft('保留消息');
    act(() => useChatStore.getState().setSessionTargetDraft('s1', { provider_id: 'p1', model_id: 'chat1', skill_ids: [], no_skill: true, generation: { ...initial, instruction: 'Unapplied guidance' } }));
    fireEvent.click(screen.getByRole('button', { name: '推理强度：低' }));
    fireEvent.click(within(screen.getByRole('dialog', { name: '推理强度' })).getByRole('button', { name: '高', exact: true }));
    await screen.findByRole('alert');
    expect(useChatStore.getState().activeSession?.generation).toEqual(initial);
    handlers.update_chat_session_target = ({ input }) => ok(session({ generation: input.generation }));
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: '高', exact: true }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(useChatStore.getState().activeSession?.generation).toEqual({ temperature: 0.4, reasoning: { effort: 'high', budget_tokens: 2048 } });
    expect(useChatStore.getState().sessionTargetDrafts.s1?.generation.instruction).toBe('Unapplied guidance');
    expect(draftBox().value).toBe('保留消息');
    fireEvent.click(screen.getByRole('button', { name: '推理强度：高' }));
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: '默认', exact: true }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(useChatStore.getState().activeSession?.generation.reasoning).toEqual({ budget_tokens: 2048 });
  });
  it('localizes skill categories while preserving filters and saves only skills after a failed attempt', async () => {
    handlers.list_installed_skills = () => [
      { skill_id: 'summary', name: '摘要', version: '1', enabled: true, call_type: 'content', manifest: { category: 'writing' } },
      { skill_id: 'research', name: '研究', version: '1', enabled: true, call_type: 'content', manifest: { category: 'data-research' } },
    ];
    handlers.update_chat_session_target = () => fail(1006013);
    await openSession();
    act(() => useChatStore.getState().setSessionTargetDraft('s1', { provider_id: 'p1', model_id: 'chat1', skill_ids: [], no_skill: true, generation: { instruction: 'Keep this draft' } }));
    fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
    fireEvent.click(await screen.findByRole('button', { name: '写作与内容', exact: true }));
    expect(screen.queryByRole('button', { name: '研究', exact: true })).toBeNull();
    act(() => setLocaleForTesting('en-US'));
    expect(screen.getByRole('button', { name: 'Writing & content', exact: true }).getAttribute('aria-pressed')).toBe('true');
    act(() => setLocaleForTesting('zh-CN'));
    fireEvent.click(screen.getByRole('button', { name: '摘要', exact: true }));
    await screen.findByRole('alert');
    expect(useChatStore.getState().activeSession?.skills).toEqual([]);
    handlers.update_chat_session_target = ({ input }) => ok(session({ skills: input.skill_ids.map((id: string) => ({ skill_id: id, version_id: `${id}@1`, content_hash: 'fixture' })), generation: input.generation, no_skill_confirmed: false }));
    fireEvent.click(screen.getByRole('button', { name: '摘要', exact: true }));
    await waitFor(() => expect(useChatStore.getState().activeSession?.skills.map(skill => skill.skill_id)).toEqual(['summary']));
    expect(useChatStore.getState().activeSession?.generation).toEqual({});
    expect(useChatStore.getState().sessionTargetDrafts.s1?.generation.instruction).toBe('Keep this draft');
    await closePanel();
    fireEvent.click(screen.getByRole('button', { name: '移除技能 摘要' }));
    await waitFor(() => expect(useChatStore.getState().activeSession?.skills).toEqual([]));
  });
});

describe('optional skills', () => {
  it('sends without a skill even when an old session has no no-skill confirmation', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session({ no_skill_confirmed: false })], next_cursor: null });
    await openSession();
    typeDraft('普通对话');
    expect(sendButton().disabled).toBe(false);
    deferSend();
    fireEvent.click(sendButton());
    expect((await sentArgs()).input.content).toBe('普通对话');
    expect(calls('update_chat_session_target')).toEqual([]);
  });

  it('1006013 on a pinned skill that became unavailable offers re-choosing the skill', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session({ skills: [{ skill_id: 'summary', version_id: 'summary@1.0.0', content_hash: 'hash' }], no_skill_confirmed: false })], next_cursor: null });
    handlers.send_chat_message = () => fail(1006013);
    await openSession();
    typeDraft('技能被禁用后发送');
    fireEvent.click(sendButton());
    await screen.findByRole('alert');
    expect(screen.getByText('会话固定的技能现在不可用。')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: '重新选择技能' }));
    await waitFor(() => expect(document.activeElement?.id).toBe('chat-skill-select'));
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
    expect(args.input).toEqual({ session_id: 's1', content: '你好', client_request_id: expect.stringMatching(UUID), foundation: defaultAiFoundation() });
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

  it('keeps observing and allows stopping again after a cancellation request fails', async () => {
    await openSession();
    const resolveAck = deferSend();
    typeDraft('写一篇长文');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    handlers.cancel_chat_generation = () => { throw new Error('lost cancellation acknowledgement'); };
    fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
    await screen.findByText('尚未确认停止。仍在观察本轮运行，请重试停止；外部动作或计费可能已经发生。');
    expect(useChatStore.getState().generation?.stopping).toBe(false);
    expect(useChatStore.getState().generation).not.toBeNull();
    expect((screen.getByRole('button', { name: '停止生成' }) as HTMLButtonElement).disabled).toBe(false);
    handlers.cancel_chat_generation = () => ok(true);
    fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
    await waitFor(() => expect(calls('cancel_chat_generation')).toHaveLength(2));
    emit(args.onEvent, { kind: 'done', message: message(2, 'assistant', '保留已生成内容', { id: 'a1', status: 'interrupted' }) });
    expect(useChatStore.getState().generation).toBeNull();
    expect(calls('send_chat_message')).toHaveLength(1);
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

  it('unmounting the page preserves the running generation', async () => {
    const view = await openSession();
    const resolveAck = deferSend();
    typeDraft('离开页面');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    view.unmount();
    await new Promise<void>(resolve => setTimeout(resolve, 0));
    expect(calls('cancel_chat_generation')).toHaveLength(0);
  });


  it('a send has no tools key and no accept_destination unless they were chosen', async () => {
    await openSession();
    deferSend();
    typeDraft('纯文本');
    fireEvent.click(sendButton());
    const { input } = await sentArgs();
    expect(Object.keys(input).sort()).toEqual(['client_request_id', 'content', 'foundation', 'session_id']);
  });

  it('announces dropped turns on the reply', async () => {
    await openSession();
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
    expect(second.input).toMatchObject({
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
    fireEvent.click(within(dialog).getByRole('button', { name: '关闭' }));
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
    fireEvent.click(within(await screen.findByRole('article', { name: '助手消息' })).getByRole('button', { name: '重新生成' }));
    fireEvent.click(within(await screen.findByRole('dialog', { name: 'Provider 目标已变化' })).getByRole('button', { name: '确认并发送' }));
    await waitFor(() => expect(calls('regenerate_chat_message')).toHaveLength(2));
    expect(calls('regenerate_chat_message')[1].input).toMatchObject({
      message_id: 'm2', client_request_id: expect.stringMatching(UUID), accept_destination: 'fp-r',
    });
    expect(calls('regenerate_chat_message')[1].input.client_request_id).not.toBe(calls('regenerate_chat_message')[0].input.client_request_id);
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
  it.each([1006008, 1006009, 1006010, 1006011, 1006015, 1006017])('send rejected with %i preserves the draft without exposing raw host errors', async code => {
    await openSession();
    handlers.send_chat_message = () => fail(code);
    typeDraft('会被拒绝');
    fireEvent.click(sendButton());
    await screen.findByRole('alert');
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
describe('message timestamps', () => {
  it('uses each message creation time rather than its update time', async () => {
    handlers.list_chat_messages = () => ok({ items: [
      message(1, 'user', '问题', { created_at: 1794182400, updated_at: 1794186000 }),
      message(2, 'assistant', '回答', { created_at: 1794182460, updated_at: 1794186060 }),
    ], next_before_seq: null });
    await openSession();
    const user = await screen.findByRole('article', { name: '用户消息' });
    const assistant = await screen.findByRole('article', { name: '助手消息' });
    expect(user.querySelector('time')?.dateTime).toBe(new Date(1794182400 * 1000).toISOString());
    expect(assistant.querySelector('time')?.dateTime).toBe(new Date(1794182460 * 1000).toISOString());
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

  it('offers regeneration only on the last reply and its paired user message', async () => {
    handlers.list_chat_messages = history;
    await openSession();
    const [first, last] = await screen.findAllByRole('article', { name: '助手消息' });
    expect(within(first).queryByRole('button', { name: '重新生成' })).toBeNull();
    expect(within(last).getByRole('button', { name: '重新生成' })).toBeTruthy();
    const [firstUser, lastUser] = screen.getAllByRole('article', { name: '用户消息' });
    expect(within(firstUser).queryByRole('button', { name: '重新生成' })).toBeNull();
    expect(within(lastUser).getByRole('button', { name: '重新生成' })).toBeTruthy();
  });

  it('is not offered when the last message is the user message', async () => {
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '问一'), message(2, 'assistant', '答一'), message(3, 'user', '问二')], next_before_seq: null });
    await openSession();
    await screen.findByText('问二');
    expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull();
  });

  it.each(['用户消息', '助手消息'])('regenerates the corresponding reply from %s and retains the user input', async role => {
    handlers.list_chat_messages = history;
    await openSession();
    const { promise, resolve } = Promise.withResolvers<unknown>();
    handlers.regenerate_chat_message = () => promise;
    const articles = await screen.findAllByRole('article', { name: role });
    fireEvent.click(within(articles[articles.length - 1]).getByRole('button', { name: '重新生成' }));
    await waitFor(() => expect(calls('regenerate_chat_message')).toHaveLength(1));
    const args = calls('regenerate_chat_message')[0];
    expect(args).toEqual({ input: { message_id: 'm4', client_request_id: expect.stringMatching(UUID), foundation: defaultAiFoundation() }, onEvent: expect.any(MockChannel) });
    expect(Object.keys(args).sort()).toEqual(['input', 'onEvent']);
    expect(screen.queryByText('答二')).toBeNull();
    expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull();

    await act(async () => { resolve(ok({ task_id: args.input.client_request_id, user_message_id: 'm3', assistant_message_id: 'a5', dropped_turns: 0, replayed: false })); });
    emit(args.onEvent, { kind: 'done', message: message(4, 'assistant', '新的答二', { id: 'a5' }) });
    await screen.findByText('新的答二');
    expect(screen.queryByText('答二')).toBeNull();
    expect(screen.getAllByRole('article', { name: '助手消息' })).toHaveLength(2);
    expect(screen.getAllByRole('article', { name: '用户消息' }).map(article => article.querySelector('.chat-message-content')?.textContent)).toEqual(['问一', '问二']);
  });

  it('a discarded regeneration restores the kept reply and says so', async () => {
    handlers.list_chat_messages = history;
    await openSession();
    const { promise, resolve } = Promise.withResolvers<unknown>();
    handlers.regenerate_chat_message = () => promise;
    const replies = await screen.findAllByRole('article', { name: '助手消息' });
    fireEvent.click(within(replies[replies.length - 1]).getByRole('button', { name: '重新生成' }));
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
    await closePanel();
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

  it('lists tools only on request and does not authorize a call while choosing them', async () => {
    await openSession();
    expect(calls('list_mcp_servers')).toHaveLength(0);
    await pickTools(/read_file/);
    expect(calls('list_mcp_tools')).toEqual([{ serverId: 'srv1' }]);
    expect(useChatStore.getState().sessionTools.s1).toHaveLength(1);
    expect(calls('send_chat_message')).toHaveLength(0);
    expect(calls('advance_chat_tool_turn')).toHaveLength(0);
  });

  it('selects at most 8 tools', async () => {
    handlers.list_mcp_tools = () => Array.from({ length: 9 }, (_, i) => tool(`tool_${i}`));
    await openSession();
    await pickTools(...Array.from({ length: 8 }, (_, i) => new RegExp(`tool_${i}\\b`)));
    expect(useChatStore.getState().sessionTools.s1).toHaveLength(8);
    expect((screen.getByRole('checkbox', { name: /tool_8\b/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('a send carries the chosen tools, the proposal shows the exact arguments, and the tool choice is spent', async () => {
    const { args } = await startToolTurn();
    expect(args.input.tools).toEqual([{ server_id: 'srv1', tool_name: 'read_file', schema_hash: 'hash-read_file' }]);
    expect(screen.getByRole('region', { name: /文件服务.*read_file/ })).toBeTruthy();
    expect(screen.getByLabelText('工具调用参数').textContent).toBe(JSON.stringify({ path: '/synthetic/a.txt', limit: 3 }, null, 2));
    expect(screen.getByText(/剩余 \d+ 秒/)).toBeTruthy();
    expect(useChatStore.getState().sessionTools.s1).toEqual([]);
    expect(calls('advance_chat_tool_turn')).toHaveLength(0);
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
    await waitFor(() => expect(screen.queryByRole('region', { name: /文件服务.*read_file/ })).toBeNull());

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
    await closePanel();
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
    expect((await screen.findByRole('alert')).textContent).toContain('工具确认已失效、已过期或与当前提案不符。请先停止本轮并核对结果，再检查工具选择；不得重发未知动作。');
    expect(screen.queryByRole('region', { name: /文件服务.*read_file/ })).toBeNull();
    expect(screen.getByText(/请先点击“停止生成”结束这一轮/)).toBeTruthy();
    expect(screen.getByRole('button', { name: '停止生成' })).toBeTruthy();
  });

  it('1006017 on send clears the tool selection and reloads the tool list on the next open', async () => {
    await openSession();
    await pickTools(/read_file/);
    expect(useChatStore.getState().sessionTools.s1).toHaveLength(1);
    await closePanel();
    handlers.send_chat_message = () => fail(1006017);
    typeDraft('工具已变化');
    fireEvent.click(sendButton());
    expect((await screen.findByRole('alert')).textContent).toContain('工具确认已失效');
    expect(useChatStore.getState().sessionTools.s1).toEqual([]);
    expect(draftBox().value).toBe('工具已变化');
    const before = calls('list_mcp_servers').length;
    await pickTools(/read_file/);
    expect(calls('list_mcp_servers').length).toBeGreaterThan(before);
    expect(useChatStore.getState().sessionTools.s1).toHaveLength(1);
  });
});

describe('deletion', () => {

  it('deleting asks first, states the limits and then deletes by session_id', async () => {
    handlers.delete_chat_session = () => ok(true);
    await openSession();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /季度复盘/ }));
    fireEvent.click(await screen.findByRole('menuitem', { name: '删除会话' }));
    const dialog = await screen.findByRole('dialog', { name: '删除会话' });
    expect(calls('delete_chat_session')).toHaveLength(0);
    fireEvent.click(within(dialog).getByRole('button', { name: '删除会话' }));
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
    await closePanel();
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

describe('account reset response isolation', () => {
  it('does not reopen a session whose create response arrives after account reset', async () => {
    const pending = Promise.withResolvers<unknown>();
    handlers.create_chat_session = () => pending.promise;
    const creating = useChatStore.getState().createSession();
    useChatStore.getState().reset();
    pending.resolve(ok(session()));
    expect(await creating).toBeNull();
    expect(useChatStore.getState().sessions).toEqual([]);
    expect(useChatStore.getState().activeSessionId).toBeNull();
  });

  it('does not let a previous account message request overwrite a fresh request for the same session', async () => {
    const old = Promise.withResolvers<unknown>();
    const fresh = Promise.withResolvers<unknown>();
    await useChatStore.getState().loadSessions();
    handlers.list_chat_messages = () => old.promise;
    const selecting = useChatStore.getState().loadLatestMessages();
    useChatStore.getState().reset();
    handlers.list_chat_messages = () => fresh.promise;
    const reopening = useChatStore.getState().loadSessions();
    await waitFor(() => expect(calls('list_chat_messages').length).toBeGreaterThanOrEqual(3));
    old.resolve(ok({ items: [message(1, 'assistant', 'old account output')], next_before_seq: null }));
    await selecting;
    expect(useChatStore.getState().messages).toEqual([]);
    expect(useChatStore.getState().messagesLoading).toBe(true);
    fresh.resolve(ok({ items: [message(2, 'assistant', 'fresh account output')], next_before_seq: null }));
    await reopening;
    expect(useChatStore.getState().messages.map(item => item.content)).toEqual(['fresh account output']);
  });

  it('ignores a rejected send from the old account after reset', async () => {
    await useChatStore.getState().loadSessions();
    const pending = Promise.withResolvers<unknown>();
    handlers.send_chat_message = () => pending.promise;
    const sending = useChatStore.getState().send('old request');
    await waitFor(() => expect(calls('send_chat_message')).toHaveLength(1));
    useChatStore.getState().reset();
    pending.resolve(fail(1006012, {fingerprint: 'old', host: 'old-provider'}));
    await sending;
    expect(useChatStore.getState().sessionPendingDestinations).toEqual({});
    expect(useChatStore.getState().sessionActionErrors).toEqual({});
    expect(useChatStore.getState().generations).toEqual({});
  });
});

describe('controlled attachment drafts', () => {
  it('keeps an attachment on send failure and transfers it to the message only after acknowledgement', async () => {
    await openSession();
    const asset = { id: 'asset-one', name: 'notes.txt', mime: 'text/plain', bytes_len: 12 };
    act(() => useChatStore.getState().setSessionAssets('s1', [asset]));
    handlers.send_chat_message = () => fail(1006008);
    fireEvent.click(sendButton());
    await screen.findByRole('alert');
    expect(useChatStore.getState().sessionAssets.s1).toEqual([asset]);
    const resolveAck = deferSend();
    fireEvent.click(sendButton());
    await waitFor(() => expect(calls('send_chat_message')).toHaveLength(2));
    const args = calls('send_chat_message')[1];
    expect(args.input.asset_ids).toEqual(['asset-one']);
    await settle(resolveAck, ackFor(args));
    expect(useChatStore.getState().sessionAssets.s1).toEqual([]);
    expect(useChatStore.getState().messages.find(item => item.id === 'u-new')?.assets).toEqual([asset]);
  });

  it('cancels a pending import without adding its late asset to the draft', async () => {
    const view = await openSession();
    const pending = Promise.withResolvers<unknown>();
    handlers.import_chat_asset = () => pending.promise;
    handlers.remove_chat_asset = () => ok({ removed: true });
    fireEvent.change(view.container.querySelector('input[type=file]')!, { target: { files: [new File(['notes'], 'notes.txt', { type: 'text/plain' })] } });
    await waitFor(() => expect(calls('import_chat_asset')).toHaveLength(1));
    fireEvent.click(screen.getByRole('button', { name: '取消导入' }));
    await act(async () => pending.resolve(ok({ id: 'cancelled-asset', name: 'notes.txt', mime: 'text/plain', bytes_len: 5 })));
    await waitFor(() => expect(calls('remove_chat_asset')).toEqual([{ assetId: 'cancelled-asset' }]));
    expect(useChatStore.getState().sessionAssets.s1 ?? []).toEqual([]);
  });

  it('finishes removing the original draft after the composer unmounts', async () => {
    const view = await openSession();
    const asset = { id: 'removed-asset', name: 'remove.txt', mime: 'text/plain', bytes_len: 5 };
    act(() => useChatStore.getState().setSessionAssets('s1', [asset]));
    const pending = Promise.withResolvers<unknown>();
    handlers.remove_chat_asset = () => pending.promise;
    fireEvent.click(screen.getByRole('button', { name: '移除 remove.txt' }));
    await waitFor(() => expect(calls('remove_chat_asset')).toHaveLength(1));
    expect(screen.queryByRole('button', { name: '取消导入' })).toBeNull();
    view.unmount();
    await act(async () => pending.resolve(ok({ removed: true })));
    expect(useChatStore.getState().sessionAssets.s1).toEqual([]);
  });

  it('retains a draft when the host refuses removal of a referenced asset', async () => {
    await openSession();
    const asset = { id: 'referenced-asset', name: 'shared.txt', mime: 'text/plain', bytes_len: 5 };
    act(() => useChatStore.getState().setSessionAssets('s1', [asset]));
    handlers.remove_chat_asset = () => fail(1006014);
    fireEvent.click(screen.getByRole('button', { name: '移除 shared.txt' }));
    await screen.findByRole('alert');
    expect(useChatStore.getState().sessionAssets.s1).toEqual([asset]);
  });
});

describe('microphone cancellation', () => {
  it.each(['cancel', 'unmount', 'reset'] as const)('releases late microphone permission after %s without transcribing', async boundary => {
    const original = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices');
    const permission = Promise.withResolvers<MediaStream>();
    const stop = vi.fn();
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia: vi.fn(() => permission.promise) } });
    try {
      handlers.list_providers = () => ok([{ id: 'voice-provider', name: 'Voice service', base_url: 'https://voice.example.invalid/v1', enabled: true, credential_status: 'stored', provider_type: 'openai_compatible' }]);
      handlers.list_models = () => ok([{ model_id: 'transcriber', model_name: 'Transcriber', model_type: 'speech-transcription' }]);
      const view = await openSession();
      await openMorePage('voice');
      await choose('转写服务', 'Voice service · voice.example.invalid');
      await choose('转写模型', 'Transcriber');
      fireEvent.click(screen.getByRole('button', { name: '开始录音' }));
      await screen.findByText('等待麦克风授权…');
      if (boundary === 'cancel') fireEvent.click(screen.getByRole('button', { name: '取消' }));
      else if (boundary === 'unmount') view.unmount();
      else act(() => useChatStore.getState().reset());
      await act(async () => permission.resolve({ getTracks: () => [{ stop }] } as unknown as MediaStream));
      expect(stop).toHaveBeenCalledTimes(1);
      expect(calls('transcribe_chat_audio')).toEqual([]);
      expect(Object.values(useChatStore.getState().sessionDrafts).every(draft => draft === '')).toBe(true);
    } finally {
      if (original) Object.defineProperty(navigator, 'mediaDevices', original); else Reflect.deleteProperty(navigator, 'mediaDevices');
    }
  });

  it.each(['failure', 'cancel', 'unmount'] as const)('handles transcription %s without losing or leaking a draft', async boundary => {
    const devices = Object.getOwnPropertyDescriptor(navigator, 'mediaDevices');
    const mediaRecorder = Object.getOwnPropertyDescriptor(globalThis, 'MediaRecorder');
    const createUrl = Object.getOwnPropertyDescriptor(URL, 'createObjectURL');
    const revokeUrl = Object.getOwnPropertyDescriptor(URL, 'revokeObjectURL');
    Object.defineProperty(URL, 'createObjectURL', { configurable: true, value: vi.fn(() => 'blob:voice-test') });
    Object.defineProperty(URL, 'revokeObjectURL', { configurable: true, value: vi.fn() });
    const stop = vi.fn();
    class Recorder {
      static isTypeSupported() { return true; }
      state = 'inactive';
      mimeType = 'audio/webm';
      ondataavailable: ((event: { data: Blob }) => void) | null = null;
      onstop: (() => void) | null = null;
      start() { this.state = 'recording'; }
      stop() { this.state = 'inactive'; this.ondataavailable?.({ data: new Blob(['recording']) }); this.onstop?.(); }
    }
    Object.defineProperty(globalThis, 'MediaRecorder', { configurable: true, value: Recorder });
    Object.defineProperty(navigator, 'mediaDevices', { configurable: true, value: { getUserMedia: vi.fn(async () => ({ getTracks: () => [{ stop }] })) } });
    try {
      handlers.list_providers = () => ok([{ id: 'voice-provider', name: 'Voice service', base_url: 'https://voice.example.invalid/v1', enabled: true, credential_status: 'stored', provider_type: 'openai_compatible' }]);
      handlers.list_models = () => ok([{ model_id: 'transcriber', model_name: 'Transcriber', model_type: 'speech-transcription' }]);
      const pending = Promise.withResolvers<unknown>();
      handlers.transcribe_chat_audio = () => pending.promise;
      handlers.cancel_chat_transcription = () => ok(true);
      const view = await openSession();
      typeDraft('keep this draft');
      await openMorePage('voice');
      await choose('转写服务', 'Voice service · voice.example.invalid');
      await choose('转写模型', 'Transcriber');
      fireEvent.click(screen.getByRole('button', { name: '开始录音' }));
      fireEvent.click(await screen.findByRole('button', { name: '结束录音' }));
      fireEvent.click(await screen.findByRole('button', { name: '转写到草稿' }));
      await waitFor(() => expect(calls('transcribe_chat_audio')).toHaveLength(1));
      if (boundary === 'cancel') fireEvent.click(screen.getByRole('button', { name: '取消' }));
      if (boundary === 'unmount') view.unmount();
      await act(async () => pending.resolve(boundary === 'failure' ? fail(1004001) : ok({ text: 'late transcript' })));
      expect(useChatStore.getState().sessionDrafts.s1).toBe('keep this draft');
      expect(stop).toHaveBeenCalledTimes(1);
      if (boundary === 'failure') {
        expect(screen.getByLabelText('录音回放').getAttribute('src')).toBe('blob:voice-test');
        expect((screen.getByRole('button', { name: '转写到草稿' }) as HTMLButtonElement).disabled).toBe(false);
      } else {
        expect(calls('cancel_chat_transcription')).toEqual([{ requestId: calls('transcribe_chat_audio')[0].input.request_id }]);
      }
      view.unmount();
    } finally {
      if (devices) Object.defineProperty(navigator, 'mediaDevices', devices); else Reflect.deleteProperty(navigator, 'mediaDevices');
      if (mediaRecorder) Object.defineProperty(globalThis, 'MediaRecorder', mediaRecorder); else Reflect.deleteProperty(globalThis, 'MediaRecorder');
      if (createUrl) Object.defineProperty(URL, 'createObjectURL', createUrl); else Reflect.deleteProperty(URL, 'createObjectURL');
      if (revokeUrl) Object.defineProperty(URL, 'revokeObjectURL', revokeUrl); else Reflect.deleteProperty(URL, 'revokeObjectURL');
    }
  });
});

it('returns from capability management to the same DG-OS session with message and unapplied target drafts intact', async () => {
 function Shell() {
  const tabs = useWorkspaceTabs();
  const location = useLocation(), navigate = useNavigate();
  return <>
   {tabs.surfaces.some(item => item.id === 'ai') && <div hidden={tabs.backgroundSurfaceId !== 'ai'}><SurfaceScope id="ai" active={tabs.activeSurfaceId === 'ai'} controller={tabs.controller}><ChatWorkbench /></SurfaceScope></div>}
   <output data-testid="capability-path">{location.pathname}</output>
   <button onClick={() => navigate(-1)}>Return to conversation</button>
  </>;
 }
 render(<MemoryRouter initialEntries={['/workbench']}><Shell /></MemoryRouter>);
 fireEvent.click(await screen.findByRole('tab', { name: /季度复盘/ }));
 await screen.findByRole('textbox', { name: '消息输入' });
 typeDraft('保留管理往返草稿');
 const prompt = draftBox();
 const original = useChatStore.getState().activeSession;
 act(() => useChatStore.getState().setSessionTargetDraft('s1', { provider_id: 'p1', model_id: 'unapplied', skill_ids: [], no_skill: true, generation: {} }));
 const draftTarget = useChatStore.getState().sessionTargetDrafts.s1;
 for (const path of ['/skills', '/mcp']) {
  if (path === '/skills') fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
  else await openMorePage('tools');
  fireEvent.click(within(await screen.findByRole('dialog')).getAllByRole('link').find(link => link.getAttribute('href') === path)!);
  await waitFor(() => expect(screen.getByTestId('capability-path').textContent).toBe(path));
  fireEvent.click(screen.getByRole('button', { name: 'Return to conversation' }));
  await waitFor(() => expect(draftBox()).toBe(prompt));
  if (screen.queryByRole('dialog')) {
   fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
   await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  }
 }
 fireEvent.click(screen.getByRole('button', { name: '会话目标' }));
 fireEvent.click(within(await screen.findByRole('dialog')).getAllByRole('link').find(link => link.getAttribute('href') === '/settings/providers')!);
 await waitFor(() => expect(screen.getByTestId('capability-path').textContent).toContain('/settings/providers'));
 handlers.list_providers = () => ok([{ id: 'p1', name: '管理后服务', enabled: true, credential_status: 'stored' }]);
 handlers.list_models = () => ok([{ id: 'after-management', model_id: 'after-management', provider_id: 'p1', model_name: '管理后模型', model_type: 'text-generation' }]);
 handlers.list_installed_skills = () => [{ skill_id: 'after-management', name: '管理后技能', version: '2', description: 'Newly installed content', enabled: true, call_type: 'content', manifest: { category: 'Content skills' } }];
 fireEvent.click(screen.getByRole('button', { name: 'Return to conversation' }));
 await waitFor(() => expect(draftBox()).toBe(prompt));
 expect(prompt.value).toBe('保留管理往返草稿');
 expect(useChatStore.getState().activeSession).toBe(original);
 expect(useChatStore.getState().sessionTargetDrafts.s1).toBe(draftTarget);
 expect(await screen.findByRole('button', { name: '管理后模型' })).toBeTruthy();
 fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
 await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
 fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
 const skillsDialog = await screen.findByRole('dialog', { name: '选择 Skills' });
 expect(await within(skillsDialog).findByRole('button', { name: '管理后技能' })).toBeTruthy();
 fireEvent.keyDown(skillsDialog, { key: 'Escape' });
 expect(calls('send_chat_message')).toEqual([]);
 expect(calls('update_chat_session_target')).toEqual([]);
 expect(calls('cancel_chat_generation')).toEqual([]);
});

it('refreshes all picker catalogs on invalidation and open, ignoring superseded responses without changing target drafts', async () => {
 await openSession();
 const draft = { provider_id: 'p1', model_id: 'unapplied-model', skill_ids: ['unapplied-skill'], no_skill: false, generation: {} };
 act(() => useChatStore.getState().setSessionTargetDraft('s1', draft));
 typeDraft('Keep this message draft');
 const oldProviders = Promise.withResolvers<unknown>(), oldModels = Promise.withResolvers<unknown>(), oldSkills = Promise.withResolvers<unknown>();
 handlers.list_providers = () => oldProviders.promise;
 handlers.list_models = () => oldModels.promise;
 handlers.list_installed_skills = () => oldSkills.promise;
 const before = calls('list_providers').length;
 act(() => window.dispatchEvent(new Event('dgos:catalog-invalidated')));
 await waitFor(() => expect(calls('list_providers').length).toBeGreaterThan(before));
 handlers.list_providers = () => ok([{ id: 'p1', name: 'Current Provider', enabled: true, credential_status: 'stored' }]);
 handlers.list_models = () => ok([{ id: 'new-model', model_id: 'new-model', model_name: 'Current model', provider_id: 'p1', model_type: 'text-generation' }]);
 handlers.list_installed_skills = () => [{ skill_id: 'new-skill', name: 'Current skill', version: '2', enabled: true, call_type: 'content', manifest: { category: 'Content skills' } }];
 fireEvent.click(screen.getByRole('button', { name: '会话目标' }));
 await screen.findByRole('button', { name: 'Current Provider' });
 await screen.findByRole('button', { name: 'Current model' });
 await act(async () => {
  oldProviders.resolve(ok([{ id: 'p1', name: 'Superseded Provider', enabled: true, credential_status: 'stored' }]));
  oldModels.resolve(ok([{ id: 'old-model', model_id: 'old-model', model_name: 'Superseded model', provider_id: 'p1', model_type: 'text-generation' }]));
  oldSkills.resolve([{ skill_id: 'old-skill', name: 'Superseded skill', version: '1', enabled: true, call_type: 'content', manifest: { category: 'Content skills' } }]);
 });
 expect(screen.queryByRole('button', { name: /Superseded/ })).toBeNull();
 expect(screen.getByRole('button', { name: 'Current model' })).toBeTruthy();
 fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
 fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
 await screen.findByRole('button', { name: 'Current skill' });
 expect(screen.queryByRole('button', { name: 'Superseded skill' })).toBeNull();
 expect(useChatStore.getState().sessionTargetDrafts.s1).toBe(draft);
 expect(draftBox().value).toBe('Keep this message draft');
 expect(calls('update_chat_session_target')).toHaveLength(0);
 expect(calls('send_chat_message')).toHaveLength(0);
});

describe('continuous conversation safety controls', () => {
  const retainedRun = (over: Args = {}) => ({ task_id: 'source-run', session_id: 's1', assistant_message_id: 'old-assistant', source_run_id: null, status: 'interrupted', partial_content: '已保留的部分回复', dispatched: true, result_unknown: false, audit_warning: null, total_tokens: null, steps: [{ id: 'step-1', ordinal: 1, kind: 'tool', phase: 'tool', status: 'completed', dispatched: true, result_unknown: false, receipt: { server_id: 'srv1', tool_name: 'write_file', result: 'confirmed', truncated: true }, created_at: 1, updated_at: 2 }], created_at: 1, updated_at: 2, ...over });

  it('a synchronous stop terminal cannot turn the clicked stop button into another send, and a newer draft survives', async () => {
    await openSession();
    const resolveAck = deferSend();
    typeDraft('第一条');
    fireEvent.click(sendButton());
    const args = await sentArgs();
    act(() => useChatStore.getState().setDraft('s1', '未发送的下一条'));
    await settle(resolveAck, ackFor(args));
    const stop = screen.getByRole('button', { name: '停止生成' }) as HTMLButtonElement;
    expect(stop.type).toBe('button');
    handlers.cancel_chat_generation = () => {
      (args.onEvent as MockChannel).emit({ kind: 'done', message: message(2, 'assistant', '停止时保留', { id: 'a1', status: 'interrupted' }) });
      return ok(true);
    };
    fireEvent.click(stop);
    await screen.findByText('停止时保留');
    expect(stop.isConnected).toBe(false);
    expect(draftBox().value).toBe('未发送的下一条');
    expect(calls('send_chat_message')).toHaveLength(1);
    expect(calls('regenerate_chat_message')).toHaveLength(0);
  });

  it('rejects an over-budget draft before dispatch and never clears the draft', async () => {
    handlers.inspect_chat_context = () => ok(chatBudget({ can_send: false, rejection: 'fixed_context_exceeds_limit' }));
    await openSession();
    typeDraft('保留超预算草稿');
    fireEvent.click(sendButton());
    await screen.findByRole('alert');
    expect(calls('send_chat_message')).toHaveLength(0);
    expect(draftBox().value).toBe('保留超预算草稿');
  });

  it('requires explicit paid-summary confirmation without inserting a normal message', async () => {
    const summary = { id: 'summary-1', source_first_seq: 1, source_last_seq: 4, source_digest: 'source-sha', content: '目标与约束', provider_id: 'p1', model_id: 'chat1', created_at: 1 };
    handlers.inspect_chat_context = () => ok(chatBudget({ model_window_tokens: null, summary }));
    handlers.summarize_chat_context = args => {
      args.onEvent.emit({ kind: 'summary_done', task_id: args.input.client_request_id, assistant_message_id: 'summary-assistant', summary, dispatched: true, result_unknown: false, audit_warning: null });
      return ok(ackFor(args, { assistant_message_id: 'summary-assistant', user_message_id: '' }));
    };
    await openSession();
    typeDraft('保留摘要期间草稿');
    fireEvent.click(screen.getByRole('button', { name: /^预计上下文：/ }));
    await waitFor(() => expect(calls('inspect_chat_context').length).toBeGreaterThan(0));
    fireEvent.click(screen.getByRole('button', { name: '整理较早对话' }));
    expect(calls('summarize_chat_context')).toHaveLength(0);
    const confirm = await screen.findByRole('dialog', { name: '整理较早对话？' });
    fireEvent.click(within(confirm).getByRole('button', { name: '允许收费摘要请求' }));
    await waitFor(() => expect(calls('summarize_chat_context')).toHaveLength(1));
    expect(calls('summarize_chat_context')[0].input).toEqual({ session_id: 's1', client_request_id: expect.stringMatching(UUID), confirmed: true });
    await waitFor(() => expect(useChatStore.getState().generation).toBeNull());
    expect(useChatStore.getState().messages).toEqual([]);
    expect(draftBox().value).toBe('保留摘要期间草稿');
  });

  it('keeps skill suggestions separate from explicit pinned subset authorization', async () => {
    const entry = { pin: { skill_id: 'writing', version: '1.2', content_hash: 'immutable-hash' }, name: '写作辅助', description: '写作与检查', category: 'writing', tags: ['写作'] };
    handlers.chat_skill_catalog = ({ input }) => ok({ items: [entry], recommendations: input.query ? [{ entry, score: 4, reasons: ['lexical_name:写作'] }] : [] });
    await openSession();
    typeDraft('写作任务');
    fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
    await screen.findByRole('checkbox', { name: /写作辅助/ });
    fireEvent.click(screen.getByRole('button', { name: '为当前草稿推荐技能' }));
    const authorize = await screen.findByRole('button', { name: '加入可选技能（不自动启用）' });
    expect(useChatStore.getState().sessionSkillAccess.s1?.authorized_skill_refs ?? []).toEqual([]);
    expect(calls('send_chat_message')).toHaveLength(0);
    fireEvent.click(authorize);
    expect(useChatStore.getState().sessionSkillAccess.s1).toEqual({ skill_mode: 'manual', authorized_skill_refs: [entry.pin] });
    fireEvent.click(screen.getByText('固定版本 1.2 与哈希'));
    expect(screen.getByText(entry.pin.content_hash)).toBeTruthy();
    fireEvent.click(screen.getByRole('checkbox', { name: '允许按需读取技能正文和资源' }));
    await closePanel();
    deferSend(); fireEvent.click(sendButton());
    expect((await sentArgs()).input).toMatchObject({ skill_mode: 'on_demand', authorized_skill_refs: [entry.pin] });
    expect(calls('update_chat_session_target')).toHaveLength(0);
  });

  it('locks optional skill authorization during pending preflight and releases it after rejection', async () => {
    const entry = { pin: { skill_id: 'writing', version: '1.2', content_hash: 'immutable-hash' }, name: '写作辅助', description: '写作与检查', category: 'writing', tags: ['写作'] };
    const pending = Promise.withResolvers<unknown>();
    handlers.chat_skill_catalog = () => ok({ items: [entry], recommendations: [] });
    handlers.inspect_chat_context = () => pending.promise;
    await openSession(); typeDraft('保留待检查草稿');
    const authorized = { skill_mode: 'on_demand' as const, authorized_skill_refs: [entry.pin] };
    act(() => useChatStore.getState().setSkillAccess('s1', authorized));
    fireEvent.click(sendButton());
    await waitFor(() => expect(useChatStore.getState().preparingSessionId).toBe('s1'));
    const remove = screen.getByRole('button', { name: '撤销可选技能：writing' }) as HTMLButtonElement;
    expect(remove.disabled).toBe(true);
    fireEvent.click(remove);
    fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
    const optional = await screen.findByRole('checkbox', { name: /写作辅助/ }) as HTMLButtonElement;
    const loading = screen.getByRole('checkbox', { name: '允许按需读取技能正文和资源' }) as HTMLButtonElement;
    expect(optional.disabled).toBe(true);
    expect(loading.disabled).toBe(true);
    fireEvent.click(optional);
    fireEvent.click(loading);
    expect(useChatStore.getState().sessionSkillAccess.s1).toEqual(authorized);
    expect(calls('send_chat_message')).toHaveLength(0);
    await act(async () => { pending.resolve(ok(chatBudget({ can_send: false }))); });
    await waitFor(() => expect(useChatStore.getState().preparingSessionId).toBeNull());
    expect(optional.disabled).toBe(false);
    expect(loading.disabled).toBe(false);
    await closePanel();
    expect(remove.disabled).toBe(false);
    fireEvent.click(remove);
    expect(useChatStore.getState().sessionSkillAccess.s1).toEqual({ skill_mode: 'on_demand', authorized_skill_refs: [] });
    expect(draftBox().value).toBe('保留待检查草稿');
    expect(calls('send_chat_message')).toHaveLength(0);
  });

  it('does not expose approval for phase or partial tool activity, scopes phases by task and ignores late activity', async () => {
    await openSession(); const resolveAck = deferSend(); typeDraft('请求工具'); fireEvent.click(sendButton());
    const args = await sentArgs(); await settle(resolveAck, ackFor(args));
    emit(args.onEvent, { kind: 'phase', task_id: 'another-task', assistant_message_id: 'a1', phase: 'tool' });
    expect(useChatStore.getState().generation?.hostPhase).toBeUndefined();
    emit(args.onEvent, { kind: 'phase', task_id: args.input.client_request_id, assistant_message_id: 'a1', phase: 'processing_result' });
    expect(screen.queryByRole('button', { name: '批准本次调用' })).toBeNull();
    expect(useChatStore.getState().generation?.hostPhase).toBe('processing_result');
    emit(args.onEvent, { kind: 'done', message: message(2, 'assistant', '终态', { id: 'a1' }), dispatched: true, result_unknown: false, audit_warning: 'audit_unconfirmed' });
    emit(args.onEvent, { kind: 'phase', task_id: args.input.client_request_id, assistant_message_id: 'a1', phase: 'final_answer' });
    typeDraft('不要重发');
    expect(sendButton().disabled).toBe(true);
    expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull();
    expect(useChatStore.getState().generation).toBeNull();
  });

  it('inspects retained checkpoints without external calls and starts a new explicit continuation identity', async () => {
    const run = retainedRun();
    handlers.list_chat_runs = () => ok({ items: [run] }); handlers.get_chat_run = () => ok(run);
    handlers.continue_chat_run = args => ok(ackFor(args, { assistant_message_id: 'continued-assistant' }));
    await openSession(); typeDraft('原草稿保留');
    await openRetainedRun();
    const next = await screen.findByRole('textbox', { name: '接下来希望助手做什么？' });
    expect(calls('continue_chat_run')).toHaveLength(0); expect(calls('advance_chat_tool_turn')).toHaveLength(0);
    fireEvent.change(next, { target: { value: '只完成下一步，不重复写入' } });
    expect((screen.getByRole('button', { name: '发起新请求' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('checkbox', { name: '按此意图发起新请求，不重放之前的操作或批准。' }));
    fireEvent.click(screen.getByRole('button', { name: '发起新请求' }));
    await waitFor(() => expect(calls('continue_chat_run')).toHaveLength(1));
    expect(calls('continue_chat_run')[0].input).toEqual({ source_task_id: 'source-run', client_request_id: expect.stringMatching(UUID), intent: '只完成下一步，不重复写入', tools: [], foundation: defaultAiFoundation() });
    expect(calls('continue_chat_run')[0].input.client_request_id).not.toBe('source-run');
    expect(draftBox().value).toBe('原草稿保留');
  });

  it('blocks resend and continuation for retained unknown tool outcomes', async () => {
    const run = retainedRun({ result_unknown: true, steps: [{ id: 'unknown-write', ordinal: 0, kind: 'tool', phase: 'tool', status: 'dispatched', dispatched: true, result_unknown: true, receipt: null, created_at: 1, updated_at: 2 }] });
    handlers.list_chat_runs = () => ok({ items: [run] }); handlers.get_chat_run = () => ok(run);
    await openSession(); typeDraft('不重复写入');
    await openRetainedRun();
    await screen.findByRole('textbox', { name: '接下来希望助手做什么？' });
    expect((screen.getByRole('button', { name: '发起新请求' }) as HTMLButtonElement).disabled).toBe(true);
    await closePanel();
    expect(sendButton().disabled).toBe(true);
    expect(calls('continue_chat_run')).toHaveLength(0); expect(calls('send_chat_message')).toHaveLength(0);
    expect(draftBox().value).toBe('不重复写入');
  });

  it('remembers only tool preference and rechecks the schema before the next send', async () => {
    await openSession(); await pickTools(/read_file/);
    fireEvent.click(screen.getByRole('checkbox', { name: '记住本会话的工具选择' }));
    await closePanel();
    const resolveAck = deferSend(); typeDraft('先读取'); fireEvent.click(sendButton());
    const args = await sentArgs(); await settle(resolveAck, ackFor(args));
    expect(useChatStore.getState().sessionTools.s1).toHaveLength(1);
    emit(args.onEvent, { kind: 'done', message: message(2, 'assistant', '完成', { id: 'a1' }) });
    handlers.list_mcp_tools = () => [tool('read_file', { schema_hash: 'changed' })];
    typeDraft('再读取'); fireEvent.click(sendButton());
    await screen.findByText('所选工具已不可用或 schema 已变化。请检查选择后再发送；本次未发出请求。');
    expect(calls('send_chat_message')).toHaveLength(1); expect(calls('advance_chat_tool_turn')).toHaveLength(0);
    expect(draftBox().value).toBe('再读取');
  });
  it('records an explicit user-reported external outcome without claiming a tool success or replaying it', async () => {
    let run = retainedRun({ result_unknown: true, steps: [{ id: 'unknown-write', ordinal: 0, kind: 'tool', phase: 'tool', status: 'dispatched', dispatched: true, result_unknown: true, receipt: null, created_at: 1, updated_at: 2 }] });
    handlers.list_chat_runs = () => ok({ items: [run] }); handlers.get_chat_run = () => ok(run);
    handlers.resolve_chat_run_step = ({ input }) => {
      run = { ...run, result_unknown: false, steps: [{ ...run.steps[0], status: 'manually_resolved', result_unknown: false, receipt: { source: 'user_report', outcome: input.outcome, note: input.note } }] };
      return ok(run);
    };
    await openSession(); typeDraft('保留原草稿');
    await openRetainedRun();
    fireEvent.click(await screen.findByText('记录第 1 项操作的核查结果'));
    expect(calls('resolve_chat_run_step')).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: '我已核查：操作已完成' }));
    fireEvent.change(screen.getByRole('textbox', { name: '你如何核查？请附上结果或参考记录。' }), { target: { value: '在外部服务管理页核对唯一回执' } });
    expect((screen.getByRole('button', { name: '保存核查结果' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('checkbox', { name: '我已在此回复之外核查实际结果，并理解这只是我的报告，不是服务出具的权威回执。' }));
    fireEvent.click(screen.getByRole('button', { name: '保存核查结果' }));
    await waitFor(() => expect(calls('resolve_chat_run_step')).toHaveLength(1));
    expect(calls('resolve_chat_run_step')[0]).toEqual({ input: { task_id: 'source-run', step_id: 'unknown-write', confirmed: true, outcome: 'completed', note: '在外部服务管理页核对唯一回执' } });
    await screen.findAllByText('用户报告的结果');
    expect(calls('advance_chat_tool_turn')).toHaveLength(0); expect(calls('continue_chat_run')).toHaveLength(0);
    await closePanel();
    expect(draftBox().value).toBe('保留原草稿');
  });

  it('does not offer model kinds or streaming capabilities that the host rejects', async () => {
    handlers.list_models = () => ok([
      { id: 'eligible', model_id: 'eligible', model_name: '可用模型', model_type: 'text-generation', max_output_tokens: 1024 },
      { id: 'legacy', model_id: 'legacy', model_name: '旧 chat 类型', model_type: 'chat' },
      { id: 'blocked', model_id: 'blocked', model_name: '禁止流式', model_type: 'text-generation', capabilities: { streaming: 'unsupported' } },
      { id: 'small', model_id: 'small', model_name: '输出不足', model_type: 'text-generation', max_output_tokens: 512 },
    ]);
    await openSession(); fireEvent.click(screen.getByRole('button', { name: '会话目标' }));
    await screen.findByRole('button', { name: '可用模型' });
    expect(screen.queryByRole('button', { name: '旧 chat 类型' })).toBeNull();
    expect(screen.queryByRole('button', { name: '禁止流式' })).toBeNull();
    expect(screen.queryByRole('button', { name: '输出不足' })).toBeNull();
  });
  it('allows only a fresh explicit next intent through provider uncertainty while keeping the original message unknown', async () => {
    const run = retainedRun({ result_unknown: true, steps: [{ id: 'provider-step', ordinal: 1, kind: 'model', phase: 'model', status: 'interrupted', dispatched: true, result_unknown: true, receipt: null, created_at: 1, updated_at: 2 }] });
    handlers.list_chat_runs = () => ok({ items: [run] }); handlers.get_chat_run = () => ok(run);
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '原请求'), message(2, 'assistant', '原请求的部分输出', { result_unknown: true, status: 'interrupted' })], next_before_seq: null });
    handlers.continue_chat_run = args => ok(ackFor(args, { assistant_message_id: 'new-intent-assistant' }));
    await openSession(); typeDraft('不能直接重发'); expect(sendButton().disabled).toBe(true);
    expect(screen.queryByRole('button', { name: '重新生成' })).toBeNull();
    await openRetainedRun();
    fireEvent.change(await screen.findByRole('textbox', { name: '接下来希望助手做什么？' }), { target: { value: '根据保留片段，执行一个新的下一步' } });
    fireEvent.click(screen.getByRole('checkbox', { name: '按此意图发起新请求，不重放之前的操作或批准。' }));
    fireEvent.click(screen.getByRole('button', { name: '发起新请求' }));
    await waitFor(() => expect(calls('continue_chat_run')).toHaveLength(1));
    expect(calls('continue_chat_run')[0].input.client_request_id).not.toBe('source-run');
    expect(useChatStore.getState().messages.find(item => item.id === 'm2')?.result_unknown).toBe(true);
    expect(calls('send_chat_message')).toHaveLength(0); expect(calls('regenerate_chat_message')).toHaveLength(0);
  });
});

describe('shared foundation context controls', () => {
  it('sends an explicitly selected context resource without requiring placeholder message text', async () => {
    await openSession();
    const context = { resource_id: 'selected-source', sha256: 'selected-sha', start: 0, max_bytes: 8, mode: 'inline' as const };
    act(() => useChatStore.getState().setFoundation('s1', { context_refs: [context], output_schema: null }));
    expect(draftBox().value).toBe('');
    expect(sendButton().disabled).toBe(false);
    const resolveAck = deferSend();
    fireEvent.click(sendButton());
    const args = await sentArgs();
    expect(args.input.content).toBe('');
    expect(args.input.foundation.context_refs).toEqual([context]);
    await settle(resolveAck, ackFor(args));
  });

  it('rejects oversized resources before reading the file or invoking import', async () => {
    await openSession();
    fireEvent.click(screen.getByRole('button', { name: '添加材料' }));
    const file = new File(['too large'], 'large.md', { type: 'text/markdown' });
    Object.defineProperty(file, 'size', { value: 1024 * 1024 + 1 });
    const read = vi.spyOn(FileReader.prototype, 'readAsDataURL');
    try {
      fireEvent.change(await screen.findByLabelText('导入参考材料'), { target: { files: [file] } });
      await screen.findByRole('alert');
      expect(read).not.toHaveBeenCalled();
      expect(calls('chat_import_ai_resource')).toHaveLength(0);
    } finally { read.mockRestore(); }
  });

  it('does not import a file that finishes reading after its resource controls unmount', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session(), session({ id: 's2', title: '另一会话' })], next_cursor: null });
    await openSession();
    fireEvent.click(screen.getByRole('button', { name: '添加材料' }));
    let reader: FileReader | undefined;
    const read = vi.spyOn(FileReader.prototype, 'readAsDataURL').mockImplementation(function (this: FileReader) { reader = this; });
    try {
      fireEvent.change(await screen.findByLabelText('导入参考材料'), { target: { files: [new File(['source'], 'source.md', { type: 'text/markdown' })] } });
      expect(reader).toBeDefined();
      await act(async () => { await useChatStore.getState().selectSession(session({ id: 's2', title: '另一会话' })); });
      await act(async () => {
        Object.defineProperty(reader!, 'result', { value: 'data:text/markdown;base64,c291cmNl' });
        reader!.onload?.call(reader!, new ProgressEvent('load') as ProgressEvent<FileReader>);
      });
      expect(calls('chat_import_ai_resource')).toHaveLength(0);
    } finally { read.mockRestore(); }
  });

  it('imports without consent to send, then previews Unicode and sends only the selected immutable bytes with the schema', async () => {
    const source = '前言\r\nA😀中e\u0301尾';
    const bytes = new TextEncoder().encode(source);
    const resource = { id: 'context-1', name: 'source.md', mime: 'text/markdown', bytes_len: bytes.length, sha256: 'immutable-context-hash', expires_at: 9999999999 };
    handlers.chat_import_ai_resource = () => ok(resource);
    handlers.chat_delete_ai_resource = () => ok(true);
    handlers.preview_chat_resource = () => ok({ base64: btoa(Array.from(bytes, byte => String.fromCharCode(byte)).join('')), mime: resource.mime, start: 0, bytes: bytes.length, total_bytes: bytes.length, truncated: false });
    await openSession(); typeDraft('使用所选资料');
    fireEvent.click(screen.getByRole('button', { name: '添加材料' }));
    fireEvent.change(await screen.findByLabelText('导入参考材料'), { target: { files: [new File([source], 'source.md', { type: 'text/markdown' })] } });
    const include = await screen.findByRole('checkbox', { name: '引用整个文件：source.md' });
    expect(useChatStore.getState().sessionFoundation.s1?.context_refs ?? []).toEqual([]);
    expect(calls('chat_import_ai_resource')[0]).toMatchObject({ sessionId: 's1', input: { name: 'source.md', mime: 'text/markdown', base64: expect.any(String) } });
    fireEvent.click(include);
    const whole = [{ resource_id: resource.id, sha256: resource.sha256, start: 0, max_bytes: bytes.length, mode: 'inline' }];
    expect(useChatStore.getState().sessionFoundation.s1.context_refs).toEqual(whole);
    fireEvent.click(screen.getByRole('button', { name: '预览并选择片段' }));
    const preview = await screen.findByRole('textbox', { name: '在只读预览中选择文字' }) as HTMLTextAreaElement;
    expect(calls('preview_chat_resource')).toEqual([{ input: { session_id: 's1', resource_id: resource.id, sha256: resource.sha256 } }]);
    expect(preview.value).toBe('前言\nA😀中e\u0301尾');
    preview.setSelectionRange(5, 9);
    fireEvent.select(preview);
    expect(useChatStore.getState().sessionFoundation.s1.context_refs).toEqual(whole);
    expect(calls('send_chat_message')).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: '引用所选文字' }));
    await choose('模型如何接收这些文字', '需要时读取');
    await closePanel();
    fireEvent.click(screen.getByRole('button', { name: '回答偏好' }));
    fireEvent.click(screen.getByText('高级参数与输出格式'));
    fireEvent.click(screen.getByRole('checkbox', { name: '按 JSON Schema 校验输出' }));
    fireEvent.change(screen.getByRole('textbox', { name: '输出 JSON Schema' }), { target: { value: '{"type":"object"}' } });
    await closePanel();
    const foundation = { ...defaultAiFoundation(), context_refs: [{ resource_id: 'context-1', sha256: resource.sha256, start: 9, max_bytes: 10, mode: 'on_demand' }], output: { kind: 'json_schema', schema: { type: 'object' } } };
    await act(async () => { await useChatStore.getState().inspectContext('s1', '使用所选资料'); });
    expect(calls('inspect_chat_context').at(-1)?.input.foundation).toEqual(foundation);
    expect(calls('send_chat_message')).toHaveLength(0);
    expect(calls('update_chat_session_target')).toHaveLength(0);
    const resolveAck = deferSend(); fireEvent.click(sendButton());
    const args = await sentArgs(); expect(args.input.foundation).toEqual(foundation);
    await settle(resolveAck, ackFor(args));
    emit(args.onEvent, { kind: 'done', message: message(2, 'assistant', '{}', { id: 'a1' }) });
    fireEvent.click(screen.getByRole('button', { name: '添加材料' }));
    fireEvent.click(screen.getByRole('button', { name: '删除托管副本…' }));
    expect(calls('chat_delete_ai_resource')).toHaveLength(0);
    fireEvent.click(screen.getByRole('button', { name: '删除托管副本', exact: true }));
    await waitFor(() => expect(screen.queryByRole('checkbox', { name: '引用整个文件：source.md' })).toBeNull());
    expect(calls('chat_delete_ai_resource')[0]).toEqual({ sessionId: 's1', resourceId: 'context-1' });
    expect(useChatStore.getState().sessionFoundation.s1.context_refs).toEqual([]);
  });

  it('blocks invalid schema before inspect/send and clears session controls on account reset', async () => {
    setLocaleForTesting('zh-CN');
    await openSession(); typeDraft('保留草稿');
    fireEvent.click(screen.getByRole('button', { name: '回答偏好' }));
    fireEvent.click(screen.getByText('高级参数与输出格式'));
    fireEvent.click(screen.getByRole('checkbox', { name: '按 JSON Schema 校验输出' }));
    fireEvent.change(screen.getByRole('textbox', { name: '输出 JSON Schema' }), { target: { value: '{invalid' } });
    await closePanel();
    const before = calls('inspect_chat_context').length;
    await act(async () => { expect(await useChatStore.getState().send('保留草稿')).toBe('rejected'); });
    expect(calls('inspect_chat_context')).toHaveLength(before);
    expect(calls('send_chat_message')).toHaveLength(0);
    expect(draftBox().value).toBe('保留草稿');
    expect(useChatStore.getState().sessionFoundation.s1.output_schema).toBe('{invalid');
    act(() => useChatStore.getState().reset());
    expect(useChatStore.getState().sessionFoundation).toEqual({});
    expect(useChatStore.getState().sessionResources).toEqual({});
  });

  it('passes current explicit resources and output schema to regeneration without reusing hidden original settings', async () => {
    setLocaleForTesting('zh-CN');
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '资料分析'), message(2, 'assistant', '旧结果')], next_before_seq: null });
    handlers.regenerate_chat_message = args => ok(ackFor(args));
    await openSession(); await screen.findByText('旧结果');
    const context = { resource_id: 'selected-source', sha256: 'selected-sha', start: 4, max_bytes: 8, mode: 'inline' as const };
    act(() => useChatStore.getState().setFoundation('s1', { context_refs: [context], output_schema: '{"type":"array"}' }));
    await act(async () => { expect(await useChatStore.getState().regenerate('m2')).toBe('started'); });
    expect(calls('regenerate_chat_message')[0].input.foundation).toEqual({ ...defaultAiFoundation(), context_refs: [context], output: { kind: 'json_schema', schema: { type: 'array' } } });
    expect(calls('send_chat_message')).toHaveLength(0);
  });
});

describe('store-owned preflight and authoritative uncertainty', () => {
  const pin = { skill_id: 'explicit-skill', version: '1.0', content_hash: 'pinned-skill-hash' };
  const runSnapshot = (taskId: string, extra: Args = {}) => ({ task_id: taskId, session_id: 's1', assistant_message_id: 'retained-assistant', source_run_id: null, status: 'completed', partial_content: 'retained result', dispatched: true, result_unknown: false, audit_warning: null, total_tokens: 4, steps: [], created_at: 1, updated_at: 2, ...extra });
  it('does not resume tool discovery from an unmounted Composer after A→B→A', async () => {
    handlers.list_chat_sessions = () => ok({ items: [session(), session({ id: 's2', title: '另一会话' })], next_cursor: null });
    await openSession();
    await pickTools(/read_file/);
    await closePanel();
    typeDraft('只发送一次');
    const oldTools = Promise.withResolvers<unknown>();
    handlers.list_mcp_tools = () => oldTools.promise;
    const before = calls('list_mcp_tools').length;
    fireEvent.click(sendButton());
    await waitFor(() => expect(calls('list_mcp_tools')).toHaveLength(before + 1));
    await act(async () => { await useChatStore.getState().selectSession(session({ id: 's2', title: '另一会话' })); });
    await act(async () => { await useChatStore.getState().selectSession(session()); });
    handlers.list_mcp_tools = () => [tool('read_file')];
    const resolveAck = deferSend();
    fireEvent.click(sendButton());
    const args = await sentArgs();
    await settle(resolveAck, ackFor(args));
    emit(args.onEvent, { kind: 'done', message: message(2, 'assistant', '唯一回复', { id: 'a1' }) });
    await act(async () => { oldTools.resolve([tool('read_file')]); });
    expect(calls('send_chat_message')).toHaveLength(1);
    expect(calls('inspect_chat_context')).toHaveLength(1);
  });


  it('keeps one paid intent through delayed inspection and A→B→A Composer remounts', async () => {
    setLocaleForTesting('zh-CN');
    const first = Promise.withResolvers<unknown>(), second = Promise.withResolvers<unknown>();
    let inspections = 0;
    handlers.inspect_chat_context = () => ++inspections === 1 ? first.promise : second.promise;
    handlers.list_chat_sessions = () => ok({ items: [session(), session({ id: 's2', title: '另一会话' })], next_cursor: null });
    await openSession(); typeDraft('只发送一次');
    const resolveAck = deferSend(); fireEvent.click(sendButton());
    await waitFor(() => expect(calls('inspect_chat_context')).toHaveLength(1));
    expect(useChatStore.getState().preparingSessionId).toBe('s1');
    await act(async () => { await useChatStore.getState().selectSession(session({ id: 's2', title: '另一会话' })); });
    await act(async () => { await useChatStore.getState().selectSession(session()); });
    expect(sendButton().disabled).toBe(true);
    let duplicate!: Promise<unknown>;
    act(() => { duplicate = useChatStore.getState().send('只发送一次'); });
    await act(async () => {
      expect(await useChatStore.getState().regenerate('m2')).toBe('rejected');
      expect(await useChatStore.getState().summarize()).toBe('rejected');
      expect(await useChatStore.getState().continueRun('source', '另一动作')).toBe('rejected');
      first.resolve(ok(chatBudget()));
    });
    const args = await sentArgs(); await settle(resolveAck, ackFor(args));
    emit(args.onEvent, { kind: 'done', message: message(2, 'assistant', '唯一回复', { id: 'a1' }) });
    await act(async () => { second.resolve(ok(chatBudget())); expect(await duplicate).toBe('rejected'); });
    expect(calls('inspect_chat_context')).toHaveLength(1);
    expect(calls('send_chat_message')).toHaveLength(1);
    expect(calls('regenerate_chat_message')).toHaveLength(0);
    expect(calls('summarize_chat_context')).toHaveLength(0);
    expect(calls('get_chat_run')).toHaveLength(0);
    expect(useChatStore.getState().preparingSessionId).toBeNull();
  });

  it('locks tool consent and voice controls during store-owned preflight after Composer remount', async () => {
    setLocaleForTesting('zh-CN');
    const pending = Promise.withResolvers<unknown>();
    handlers.inspect_chat_context = () => pending.promise;
    handlers.list_chat_sessions = () => ok({ items: [session(), session({ id: 's2', title: '另一会话' })], next_cursor: null });
    await openSession();
    await pickTools(/read_file/); await closePanel();
    const selected = useChatStore.getState().sessionTools.s1;
    typeDraft('保留工具选择'); fireEvent.click(sendButton());
    await waitFor(() => expect(useChatStore.getState().preparingSessionId).toBe('s1'));
    await act(async () => { await useChatStore.getState().selectSession(session({ id: 's2', title: '另一会话' })); });
    await act(async () => { await useChatStore.getState().selectSession(session()); });
    const remove = screen.getByRole('button', { name: '移除工具 read_file' }) as HTMLButtonElement;
    expect(remove.disabled).toBe(true); fireEvent.click(remove);
    await openMorePage('tools');
    const toolChoice = await screen.findByRole('checkbox', { name: /read_file/ }) as HTMLButtonElement;
    const remember = screen.getByRole('checkbox', { name: '记住本会话的工具选择' }) as HTMLButtonElement;
    expect(toolChoice.disabled).toBe(true); expect(remember.disabled).toBe(true);
    fireEvent.click(toolChoice); fireEvent.click(remember);
    expect(useChatStore.getState().sessionTools.s1).toEqual(selected);
    expect(useChatStore.getState().sessionRememberTools.s1 ?? false).toBe(false);
    await openMorePage('voice');
    expect((screen.getByRole('combobox', { name: '转写服务' }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('combobox', { name: '转写模型' }) as HTMLButtonElement).disabled).toBe(true);
    await act(async () => { pending.resolve(ok(chatBudget({ can_send: false }))); });
    await waitFor(() => expect(useChatStore.getState().preparingSessionId).toBeNull());
    expect((screen.getByRole('combobox', { name: '转写服务' }) as HTMLButtonElement).disabled).toBe(false);
    await closePanel();
    expect(remove.disabled).toBe(false);
    expect(draftBox().value).toBe('保留工具选择');
    expect(calls('send_chat_message')).toHaveLength(0);
    expect(calls('transcribe_chat_audio')).toHaveLength(0);
  });

  it('releases rejected and failed preflight reservations without dispatching', async () => {
    setLocaleForTesting('zh-CN');
    await openSession();
    handlers.inspect_chat_context = () => ok(chatBudget({ can_send: false }));
    await act(async () => { expect(await useChatStore.getState().send('超限')).toBe('rejected'); });
    expect(useChatStore.getState().preparingSessionId).toBeNull();
    handlers.inspect_chat_context = () => Promise.reject(new Error('transport failure'));
    await act(async () => { expect(await useChatStore.getState().send('检查失败')).toBe('rejected'); });
    expect(useChatStore.getState().preparingSessionId).toBeNull();
    expect(calls('send_chat_message')).toHaveLength(0);
  });

  it('does not let a stale preflight finally release a new account reservation', async () => {
    setLocaleForTesting('zh-CN');
    const oldBudget = Promise.withResolvers<unknown>(), newBudget = Promise.withResolvers<unknown>();
    let inspection = 0;
    handlers.inspect_chat_context = () => ++inspection === 1 ? oldBudget.promise : newBudget.promise;
    await openSession();
    let oldIntent!: Promise<unknown>, newIntent!: Promise<unknown>;
    act(() => { oldIntent = useChatStore.getState().send('旧账户意图'); });
    await waitFor(() => expect(calls('inspect_chat_context')).toHaveLength(1));
    await act(async () => { useChatStore.getState().reset(); await useChatStore.getState().loadSessions(); await useChatStore.getState().selectSession(session()); });
    act(() => { newIntent = useChatStore.getState().send('新账户意图'); });
    await waitFor(() => expect(calls('inspect_chat_context')).toHaveLength(2));
    await act(async () => { oldBudget.resolve(ok(chatBudget())); expect(await oldIntent).toBe('rejected'); });
    expect(useChatStore.getState().preparingSessionId).toBe('s1');
    await act(async () => { newBudget.resolve(ok(chatBudget({ can_send: false }))); expect(await newIntent).toBe('rejected'); });
    expect(useChatStore.getState().preparingSessionId).toBeNull();
    expect(calls('send_chat_message')).toHaveLength(0);
  });

  it.each(['manual', 'on_demand'] as const)('normalizes %s skill preferences for inspect, send, regenerate and continue', async mode => {
    setLocaleForTesting('zh-CN');
    handlers.list_chat_messages = () => ok({ items: [message(1, 'user', '原问题'), message(2, 'assistant', '原回答')], next_before_seq: null });
    handlers.send_chat_message = args => ok(ackFor(args));
    handlers.regenerate_chat_message = args => ok(ackFor(args));
    handlers.continue_chat_run = args => ok(ackFor(args));
    handlers.get_chat_run = () => ok(runSnapshot('source-run'));
    await openSession(); await screen.findByText('原回答');
    act(() => useChatStore.getState().setSkillAccess('s1', { skill_mode: mode, authorized_skill_refs: [pin] }));
    const wire = { skill_mode: mode, authorized_skill_refs: mode === 'on_demand' ? [pin] : [] };
    await act(async () => { await useChatStore.getState().inspectContext('s1', '新问题'); });
    expect(calls('inspect_chat_context').at(-1)?.input).toMatchObject(wire);
    await act(async () => { expect(await useChatStore.getState().regenerate('m2')).toBe('started'); });
    const regenerated = calls('regenerate_chat_message')[0];
    expect(regenerated.input).toMatchObject(wire);
    emit(regenerated.onEvent, { kind: 'done', message: message(2, 'assistant', '新回答', { id: 'a1' }) });
    await act(async () => { expect(await useChatStore.getState().continueRun('source-run', '新的明确动作')).toBe('started'); });
    const continued = calls('continue_chat_run')[0];
    expect(continued.input).toMatchObject(wire);
    emit(continued.onEvent, { kind: 'done', message: message(4, 'assistant', '继续回复', { id: 'a1' }) });
    await act(async () => { expect(await useChatStore.getState().send('普通发送')).toBe('started'); });
    expect(calls('send_chat_message')[0].input).toMatchObject(wire);
    expect(calls('inspect_chat_context').at(-1)?.input).toMatchObject(wire);
    expect(useChatStore.getState().sessionSkillAccess.s1).toEqual({ skill_mode: mode, authorized_skill_refs: [pin] });
  });

  it.each(['list', 'inspect'] as const)('resolves only the exact lost request from safe terminal %s evidence', async readMode => {
    setLocaleForTesting('zh-CN');
    handlers.send_chat_message = () => { throw new Error('lost acknowledgement'); };
    await openSession();
    await act(async () => { expect(await useChatStore.getState().send('原请求')).toBe('rejected'); });
    const taskId = calls('send_chat_message')[0].input.client_request_id;
    expect(useChatStore.getState().sessionUnknown.s1).toBe(true);
    await act(async () => { await useChatStore.getState().listRuns('s1'); });
    expect(useChatStore.getState().sessionUnknown.s1).toBe(true);
    handlers.get_chat_run = () => ok(runSnapshot('unrelated-run'));
    await act(async () => { await useChatStore.getState().inspectRun('unrelated-run'); });
    expect(useChatStore.getState().sessionUnknown.s1).toBe(true);
    let observed = runSnapshot(taskId, { status: 'running' });
    handlers.list_chat_runs = () => ok({ items: [observed] });
    handlers.get_chat_run = () => ok(observed);
    const read = async () => { if (readMode === 'list') await useChatStore.getState().listRuns('s1'); else await useChatStore.getState().inspectRun(taskId); };
    for (const extra of [{ status: 'running' }, { status: 'completed', result_unknown: true }, { status: 'completed', audit_warning: 'audit_unconfirmed' }, { status: 'completed', steps: [{ id: 'uncertain-tool', kind: 'tool', status: 'dispatched', dispatched: true, result_unknown: false, receipt: null }] }]) {
      observed = runSnapshot(taskId, extra);
      await act(read);
      expect(useChatStore.getState().sessionUnknown.s1).toBe(true);
    }
    observed = runSnapshot(taskId);
    await act(read);
    expect(useChatStore.getState().sessionUnknown.s1).toBe(false);
    expect(calls('send_chat_message')).toHaveLength(1);
    handlers.send_chat_message = args => ok(ackFor(args));
    await act(async () => { expect(await useChatStore.getState().send('核实后的新意图')).toBe('started'); });
    expect(calls('send_chat_message')).toHaveLength(2);
    expect(calls('send_chat_message')[1].input.client_request_id).not.toBe(taskId);
  });

  it('does not clear a different genuinely unknown run when a lost acknowledgement is reconciled', async () => {
    setLocaleForTesting('zh-CN');
    handlers.send_chat_message = () => { throw new Error('lost acknowledgement'); };
    await openSession();
    await act(async () => { await useChatStore.getState().send('原请求'); });
    const taskId = calls('send_chat_message')[0].input.client_request_id;
    handlers.list_chat_runs = () => ok({ items: [runSnapshot('unknown-effect', { result_unknown: true }), runSnapshot(taskId)] });
    await act(async () => { await useChatStore.getState().listRuns('s1'); });
    handlers.get_chat_run = () => ok(runSnapshot(taskId));
    await act(async () => { await useChatStore.getState().inspectRun(taskId); });
    expect(useChatStore.getState().sessionUnknown.s1).toBe(true);
    await act(async () => { expect(await useChatStore.getState().send('禁止重发')).toBe('rejected'); });
    expect(calls('send_chat_message')).toHaveLength(1);
  });
});

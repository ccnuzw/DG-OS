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
import { useCapabilityEntry } from '../src/components/chat/RuntimeNavigation';
import { setLocaleForTesting } from '../src/i18n';

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

async function pickTools(...names: RegExp[]) {
  const toggle = screen.getByRole('button', { name: /MCP 工具（可选/ });
  if (toggle.getAttribute('aria-expanded') !== 'true') fireEvent.click(toggle);
  for (const name of names) fireEvent.click(await screen.findByRole('checkbox', { name }));
}

const pickerLabel = (selected: number) => screen.getByRole('button', { name: new RegExp(`MCP 工具（可选，已选 ${selected} / 8）`) });

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
    handlers.list_models = ({ providerId }) => ok([{ id: 'm', model_id: providerId === 'p2' ? 'modelB' : 'chat1', provider_id: providerId, model_name: providerId === 'p2' ? '模型 B' : 'Chat', model_type: 'chat' }]);
    handlers.update_chat_session_target = ({ input }) => ok(session({ id: input.session_id, title: '会话 A', provider_id: input.provider_id, model_id: input.model_id }));
    renderPage();
    await screen.findByRole('tab', { name: '会话 A' });
    typeDraft('draft A');
    await choose('Provider', '服务 B');
    await choose('模型', '模型 B');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: '会话设置' }));
    fireEvent.change(screen.getByRole('textbox', { name: '自定义指导' }), { target: { value: 'draft guidance A' } });
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
    expect(dialog.textContent).toContain('会保留已生成内容和输入草稿');
    fireEvent.click(within(dialog).getByRole('button', { name: '关闭会话' }));
    await waitFor(() => expect(calls('cancel_chat_generation')).toEqual([{ taskId: args.input.client_request_id }]));
    expect(screen.queryByRole('tab', { name: '会话 A' })).toBeNull();
    await useChatStore.getState().selectSession(session());
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
    handlers.list_models = () => ok([{ id: 'm1', model_id: 'chat1', model_name: '对话模型', model_type: 'chat' }, { id: 'm2', model_id: 'chat2', model_name: '第二模型', model_type: 'chat' }]);
    handlers.update_chat_session_target = ({ input }) => ok(session({ provider_id: input.provider_id, model_id: input.model_id, skills: input.skill_ids.map((id: string) => ({ skill_id: id, version_id: `${id}@1`, content_hash: 'fixture' })), no_skill_confirmed: input.no_skill, generation: input.generation }));
    await openSession();
    await choose('模型', '第二模型');
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: '会话目标' })));
    fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
    fireEvent.click(await within(screen.getByRole('dialog')).findByRole('button', { name: 'summary', exact: true }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
    fireEvent.click(await within(screen.getByRole('dialog')).findByRole('button', { name: 'writing', exact: true }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: '选择 Skills' }));
    fireEvent.click(await screen.findByRole('button', { name: '上移 writing' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(useChatStore.getState().activeSession?.skills.map(skill => skill.skill_id)).toEqual(['writing', 'summary']);
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: '选择 Skills' })));
    fireEvent.click(screen.getByRole('button', { name: '会话设置' }));
    fireEvent.change(screen.getByRole('textbox', { name: '自定义指导' }), { target: { value: 'Preserve sources' } });
    expect(calls('update_chat_session_target')).toHaveLength(4);
    expect(useChatStore.getState().activeSession?.model_id).toBe('chat2');
    fireEvent.click(screen.getByRole('button', { name: '应用目标' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(useChatStore.getState().activeSession?.skills.map(skill => skill.skill_id)).toEqual(['writing', 'summary']);
    expect(useChatStore.getState().activeSession?.generation.instruction).toBe('Preserve sources');
  });
  it('keeps the current model and draft on save failure, then closes only after a successful retry', async () => {
    handlers.list_models = () => ok([{ id: 'm2', model_id: 'chat2', model_name: '第二模型', model_type: 'chat' }]);
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
    handlers.list_chat_sessions = () => ok({ items: [session()], next_cursor: null });
    await openSession();
    fireEvent.click(screen.getByRole('button', { name: '会话设置' }));
    fireEvent.change(screen.getByRole('textbox', { name: '自定义指导' }), { target: { value: 'Needs retry' } });
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: '应用目标' }));
    await within(screen.getByRole('dialog')).findByRole('alert');
    expect((screen.getByRole('textbox', { name: '自定义指导' }) as HTMLTextAreaElement).value).toBe('Needs retry');
    expect(useChatStore.getState().activeSession?.generation.instruction).toBeUndefined();
    handlers.update_chat_session_target = ({ input }) => ok(session({ generation: input.generation }));
    fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: '应用目标' }));
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
    expect(useChatStore.getState().activeSession?.generation.reasoning).toBeNull();
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
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(useChatStore.getState().activeSession?.skills.map(skill => skill.skill_id)).toEqual(['summary']);
    expect(useChatStore.getState().activeSession?.generation).toEqual({});
    expect(useChatStore.getState().sessionTargetDrafts.s1?.generation.instruction).toBe('Keep this draft');
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
    expect(Object.keys(input).sort()).toEqual(['client_request_id', 'content', 'session_id']);
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
    expect(args).toEqual({ input: { message_id: 'm4', client_request_id: expect.stringMatching(UUID) }, onEvent: expect.any(MockChannel) });
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
      fireEvent.click(screen.getByRole('button', { name: '语音输入' }));
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
      fireEvent.click(screen.getByRole('button', { name: '语音输入' }));
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
  const skills = useCapabilityEntry('/skills'), mcp = useCapabilityEntry('/mcp');
  return <>
   {tabs.surfaces.some(item => item.id === 'ai') && <div hidden={tabs.backgroundSurfaceId !== 'ai'}><SurfaceScope id="ai" active={tabs.activeSurfaceId === 'ai'} controller={tabs.controller}><ChatWorkbench /></SurfaceScope></div>}
   <output data-testid="capability-path">{location.pathname}:{location.pathname === '/skills' ? skills.runtime : mcp.runtime}</output>
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
 for (const [trigger, path] of [['选择 Skills', '/skills'], [/MCP 工具/, '/mcp']] as const) {
  fireEvent.click(screen.getByRole('button', { name: trigger }));
  fireEvent.click(within(await screen.findByRole('dialog')).getAllByRole('link').find(link => link.getAttribute('href') === path)!);
  await waitFor(() => expect(screen.getByTestId('capability-path').textContent).toBe(`${path}:dgos`));
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
 handlers.list_models = () => ok([{ id: 'after-management', model_id: 'after-management', provider_id: 'p1', model_name: '管理后模型', model_type: 'chat' }]);
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
 handlers.list_models = () => ok([{ id: 'new-model', model_id: 'new-model', model_name: 'Current model', provider_id: 'p1', model_type: 'chat' }]);
 handlers.list_installed_skills = () => [{ skill_id: 'new-skill', name: 'Current skill', version: '2', enabled: true, call_type: 'content', manifest: { category: 'Content skills' } }];
 fireEvent.click(screen.getByRole('button', { name: '会话目标' }));
 await screen.findByRole('button', { name: 'Current Provider' });
 await screen.findByRole('button', { name: 'Current model' });
 await act(async () => {
  oldProviders.resolve(ok([{ id: 'p1', name: 'Superseded Provider', enabled: true, credential_status: 'stored' }]));
  oldModels.resolve(ok([{ id: 'old-model', model_id: 'old-model', model_name: 'Superseded model', provider_id: 'p1', model_type: 'chat' }]));
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

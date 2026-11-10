import { chatBudget } from './helpers/chatBudget';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import Composer from '../src/components/chat/Composer';
import MessageList from '../src/components/chat/MessageList';
import { useChatStore, type ChatMessage, type ChatSession, type ChatStreamEvent, type SendAck, type SendOutcome } from '../src/stores/chatStore';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';
import { MockChannel } from './helpers/mockChannel';

vi.mock('@tauri-apps/api/core', async () => {
  // Vitest hoists this factory before static imports, so the controllable Channel must load inside it.
  const { MockChannel: Channel } = await import('./helpers/mockChannel');
  return { invoke: vi.fn(), Channel };
});

const sessions: ChatSession[] = ['s1', 's2'].map(id => ({
  id, title: id, title_source: 'user', provider_id: 'p1', model_id: 'chat1', skills: [],
  generation: {}, no_skill_confirmed: true, created_at: 1, updated_at: 1,
}));
const ok = (data: unknown) => ({ success: true, data });
const message = (sessionId: string, content: string, id = `history-${sessionId}`): ChatMessage => ({
  id, session_id: sessionId, seq: 1, role: 'assistant', content, status: 'completed', task_id: null,
  finish_reason: 'stop', error_code: null, total_tokens: 10, result_unknown: false, created_at: 1, updated_at: 1,
});
const emit = (event: ChatStreamEvent) => act(() => MockChannel.last().emit(event));
let acknowledge: ((ack: SendAck) => void) | undefined;

beforeEach(async () => {
  MockChannel.reset();
  useChatStore.getState().reset();
  acknowledge = undefined;
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    switch (command) {
      case 'list_chat_sessions': return ok({ items: sessions, next_cursor: null });
      case 'inspect_chat_context': return ok(chatBudget());
      case 'list_chat_messages': {
        const { input } = args as { input: { session_id: string } };
        return ok({ items: [message(input.session_id, `历史消息 ${input.session_id}`)], next_before_seq: null });
      }
      case 'send_chat_message': {
        const { input } = args as { input: { client_request_id: string } };
        const { promise, resolve } = Promise.withResolvers<unknown>();
        acknowledge = ack => resolve(ok({ ...ack, task_id: input.client_request_id }));
        return await promise;
      }
      case 'list_providers': return ok([{ id: 'p1', name: '合成服务', enabled: true, credential_status: 'stored' }]);
      case 'list_models': return ok([{ id: 'm1', model_id: 'chat1', provider_id: 'p1', model_name: '对话模型', model_type: 'text-generation' }]);
      case 'list_installed_skills': return [];
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });
  await useChatStore.getState().loadSessions();
});

afterEach(() => {
  cleanup();
  useChatStore.getState().reset();
  vi.restoreAllMocks();
});

async function selectSession(index: number) {
  await act(async () => { await useChatStore.getState().selectSession(sessions[index]); });
}

async function acceptSend() {
  await waitFor(() => expect(acknowledge).toBeTypeOf('function'));
  await act(async () => {
    acknowledge!({ task_id: '', user_message_id: 'u1', assistant_message_id: 'a1', dropped_turns: 0, replayed: false });
  });
}

async function startGeneration() {
  let sending!: Promise<SendOutcome>;
  act(() => { sending = useChatStore.getState().send('继续回答'); });
  await acceptSend();
  expect(await sending).toBe('started');
}

function mountReadingArea() {
  let height = 1200;
  const positions = new WeakMap<HTMLElement, number>();
  // JSDOM has no layout or browser scroll clamping. Model a 300px viewport with real scroll limits.
  vi.spyOn(HTMLElement.prototype, 'scrollHeight', 'get').mockImplementation(() => height);
  vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(300);
  vi.spyOn(HTMLElement.prototype, 'scrollTop', 'get').mockImplementation(function (this: HTMLElement) {
    return positions.get(this) ?? 0;
  });
  vi.spyOn(HTMLElement.prototype, 'scrollTop', 'set').mockImplementation(function (this: HTMLElement, top: number) {
    positions.set(this, Math.max(0, Math.min(top, height - this.clientHeight)));
  });
  const view = render(<MessageList />);
  const viewport = screen.getByRole('region', { name: '消息' });
  return {
    viewport,
    grow: (next: number) => { height = next; },
    scroll: (top: number) => fireEvent.scroll(viewport, { target: { scrollTop: top } }),
  };
}

function mountComposer() {
  const controller = createWorkspaceController();
  const tree = (active = true, hidden = false, covered = false) => <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
    <input aria-label="外部输入" />
    {covered && <div role="dialog" aria-label="覆盖层"><button type="button">覆盖层控件</button></div>}
    <div hidden={hidden}>
      <SurfaceScope id="ai" active={active} visible={!hidden} controller={controller}><Composer /></SurfaceScope>
    </div>
  </MemoryRouter>;
  const view = render(tree());
  const input = screen.getByRole('textbox', { name: '消息输入' }) as HTMLTextAreaElement;
  const external = screen.getByRole('textbox', { name: '外部输入' });
  const focus = vi.spyOn(input, 'focus');
  return { input, external, focus, activity: (active: boolean, hidden = false, covered = false) => view.rerender(tree(active, hidden, covered)) };
}

describe('chat reading position', () => {
  it('opens a second session at the bottom and restores the first session at exactly zero', async () => {
    const view = mountReadingArea();
    expect(view.viewport.scrollTop).toBe(900);
    view.scroll(0);
    await selectSession(1);
    expect(screen.getByText('历史消息 s2')).toBeTruthy();
    expect(view.viewport.scrollTop).toBe(900);
    view.scroll(900);
    await selectSession(0);
    expect(screen.getByText('历史消息 s1')).toBeTruthy();
    expect(view.viewport.scrollTop).toBe(0);
  });

  it('releases bottom-follow as soon as the reader scrolls upward', async () => {
    const view = mountReadingArea();
    expect(view.viewport.scrollTop).toBe(900);

    view.scroll(870);

    expect(view.viewport.scrollTop).toBe(870);
    expect(screen.getByRole('button', { name: '回到底部' })).toBeTruthy();
  });

  it('keeps the reader at the same position while streamed text increases the scroll height', async () => {
    const view = mountReadingArea();
    await startGeneration();
    view.scroll(275);
    view.grow(1500);
    emit({ kind: 'delta', assistant_message_id: 'a1', text: '第一段后台回复' });
    expect(screen.getByText('第一段后台回复')).toBeTruthy();
    expect(view.viewport.scrollTop).toBe(275);
    view.grow(1800);
    emit({ kind: 'delta', assistant_message_id: 'a1', text: '，接着一段' });
    expect(screen.getByText('第一段后台回复，接着一段')).toBeTruthy();
    expect(view.viewport.scrollTop).toBe(275);
  });
});

describe('chat composer completion focus and send ownership', () => {
  it('returns focus to the composer once when the active generation completes', async () => {
    const view = mountComposer();
    await startGeneration();
    view.external.focus();
    view.external.blur();
    view.focus.mockClear();
    emit({ kind: 'done', message: message('s1', '回答完成', 'a1') });
    expect(view.input.disabled).toBe(false);
    expect(view.focus).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(view.input);
  });

  it.each([
    ['inactive', false, false],
    ['hidden', true, false],
    ['covered', false, true],
  ] as const)('never attempts completion focus while %s and does not defer it until reactivation', async (_label, hidden, covered) => {
    const view = mountComposer();
    await startGeneration();
    view.activity(false, hidden, covered);
    const external = covered ? screen.getByRole('button', { name: '覆盖层控件' }) : view.external;
    external.focus();
    view.focus.mockClear();
    emit({ kind: 'done', message: message('s1', '后台回答完成', 'a1') });
    expect(view.input.disabled).toBe(false);
    expect(view.focus).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(external);
    // Return focus to another persistent control before removing the covering dialog.
    view.external.focus();
    view.activity(true);
    expect(view.focus).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(view.external);
  });

  it('keeps the current session draft when a different session send is acknowledged late', async () => {
    const view = mountComposer();
    fireEvent.change(view.input, { target: { value: '发送给 s1' } });
    fireEvent.click(screen.getByRole('button', { name: '发送' }));
    await waitFor(() => expect(acknowledge).toBeTypeOf('function'));
    await selectSession(1);
    fireEvent.change(view.input, { target: { value: '留给 s2 的草稿' } });
    await acceptSend();
    expect(view.input.value).toBe('留给 s2 的草稿');
    expect(useChatStore.getState().sessionDrafts.s2).toBe('留给 s2 的草稿');
    expect(useChatStore.getState().sessionDrafts.s1).toBe('');
    await selectSession(0);
    await selectSession(1);
    expect(view.input.value).toBe('留给 s2 的草稿');
  });
});

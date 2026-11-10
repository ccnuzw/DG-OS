import { chatBudget } from './helpers/chatBudget';
import { Profiler } from 'react';
import { act, cleanup, render, screen } from '@testing-library/react';
import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from 'vitest';
import MessageList from '../src/components/chat/MessageList';
import { useChatStore, type ChatSession, type ChatStreamEvent, type ChatToolProposal } from '../src/stores/chatStore';
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
const proposal = (id = 'proposal-1', seconds = 5): ChatToolProposal => ({
  id, server_id: 'srv1', server_name: '文件服务', tool_name: 'read_file', schema_hash: 'schema',
  arguments: { path: '/synthetic/example.txt' }, expires_at: Date.now() / 1000 + seconds,
});
const emit = (event: ChatStreamEvent) => act(() => MockChannel.last().emit(event));
const arrive = (value = proposal()) => emit({ kind: 'tool_proposal', assistant_message_id: 'a1', proposal: value });
const heading = () => screen.getByRole('heading', { name: '确认工具调用', hidden: true });
const remaining = () => screen.getByText(/剩余 \d+ 秒/).textContent;
const originalFocus = HTMLElement.prototype.focus;
let focus: MockInstance<HTMLElement['focus']>;
const approvalFocusAttempts = () => focus.mock.contexts.filter(element => element instanceof HTMLElement && element.id === 'chat-proposal-heading');

beforeEach(async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-11-05T00:00:00Z'));
  MockChannel.reset();
  useChatStore.getState().reset();
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    switch (command) {
      case 'list_chat_sessions': return ok({ items: sessions, next_cursor: null });
      case 'list_chat_messages': return ok({ items: [], next_before_seq: null });
      case 'inspect_chat_context': return ok(chatBudget());
      case 'send_chat_message': {
        const { input } = args as { input: { client_request_id: string } };
        return ok({ task_id: input.client_request_id, user_message_id: 'u1', assistant_message_id: 'a1', dropped_turns: 0, replayed: false });
      }
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });
  await useChatStore.getState().loadSessions();
  // Browsers reject focus into display:none content; retain the attempt separately from actual focus.
  focus = vi.spyOn(HTMLElement.prototype, 'focus').mockImplementation(function (this: HTMLElement, options?: FocusOptions) {
    if (!this.closest('[hidden]')) originalFocus.call(this, options);
  });
});

afterEach(() => {
  cleanup();
  useChatStore.getState().reset();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

async function startGeneration() {
  await act(async () => {
    expect(await useChatStore.getState().send('读取文件', [{ server_id: 'srv1', tool_name: 'read_file', schema_hash: 'schema' }])).toBe('started');
  });
}

function mount(active = true, hidden = false) {
  const controller = createWorkspaceController();
  const commits = vi.fn();
  const tree = (nextActive: boolean, nextHidden: boolean) => <>
    <input aria-label="外部输入" />
    <div hidden={nextHidden}>
      <SurfaceScope id="ai" active={nextActive} controller={controller}>
        <Profiler id="chat" onRender={commits}><MessageList /></Profiler>
      </SurfaceScope>
    </div>
  </>;
  const view = render(tree(active, hidden));
  const external = screen.getByRole('textbox', { name: '外部输入' });
  external.focus();
  focus.mockClear();
  return { ...view, external, commits, activity: (nextActive: boolean, nextHidden = false) => view.rerender(tree(nextActive, nextHidden)) };
}

async function selectSession(index: number) {
  await act(async () => { await useChatStore.getState().selectSession(sessions[index]); });
}

describe('chat approval surface activity', () => {
  it('focuses a new approval once in the active current session, without refocusing on countdown or stream updates', async () => {
    const view = mount();
    await startGeneration();
    arrive();
    expect(approvalFocusAttempts()).toHaveLength(1);
    expect(document.activeElement).toBe(heading());
    view.external.focus();
    act(() => vi.advanceTimersByTime(1000));
    emit({ kind: 'delta', assistant_message_id: 'a1', text: '仍然收到流事件' });
    expect(remaining()).toContain('4');
    expect(screen.getByText('仍然收到流事件')).toBeTruthy();
    expect(approvalFocusAttempts()).toHaveLength(1);
    expect(document.activeElement).toBe(view.external);
    arrive(proposal('proposal-2'));
    expect(approvalFocusAttempts()).toHaveLength(2);
    expect(document.activeElement).toBe(heading());
  });

  it.each(['dialog', 'alertdialog', 'listbox', 'menu'])('preserves a nested %s focus when approval arrives and does not defer a focus steal', async role => {
    const view = mount();
    await startGeneration();
    const overlay = document.createElement('div');
    overlay.setAttribute('role', role);
    overlay.setAttribute('data-state', 'open');
    overlay.tabIndex = -1;
    const field = document.createElement('input');
    overlay.appendChild(field);
    document.body.appendChild(overlay);
    try {
      field.focus();
      focus.mockClear();
      arrive();
      expect(document.activeElement).toBe(field);
      expect(approvalFocusAttempts()).toHaveLength(0);
      act(() => vi.advanceTimersByTime(1000));
      emit({ kind: 'delta', assistant_message_id: 'a1', text: '后续流文本' });
      expect(document.activeElement).toBe(field);
      overlay.remove();
      view.external.focus();
      act(() => vi.advanceTimersByTime(1000));
      expect(document.activeElement).toBe(view.external);
      expect(approvalFocusAttempts()).toHaveLength(0);
    } finally { overlay.remove(); }
  });

  it('notifies approval phase without announcing stream deltas or countdown ticks', async () => {
    mount();
    await startGeneration();
    arrive();
    const notice = screen.getAllByRole('status').find(node => node.classList.contains('sr-only'))!;
    const before = notice.textContent;
    act(() => vi.advanceTimersByTime(1000));
    emit({ kind: 'delta', assistant_message_id: 'a1', text: '无需连续播报' });
    expect(notice.textContent).toBe(before);
    expect(notice.textContent).not.toContain('无需连续播报');
    expect(screen.getByText(/剩余 \d+ 秒/).closest('[role="status"], [aria-live="polite"], [aria-live="assertive"]')).toBeNull();
  });

  it.each([
    ['hidden surface', true],
    ['covered surface', false],
  ] as const)('receives approval and stream events on a %s without attempting focus or refreshing the countdown', async (_label, hidden) => {
    const view = mount(false, hidden);
    await startGeneration();
    const pending = proposal();
    arrive(pending);
    emit({ kind: 'delta', assistant_message_id: 'a1', text: '后台流文本' });
    expect(useChatStore.getState().generations.s1.proposal).toEqual(pending);
    expect(useChatStore.getState().generation?.text).toBe('后台流文本');
    expect(screen.getByText('后台流文本')).toBeTruthy();
    const before = remaining();
    const commits = view.commits.mock.calls.length;
    act(() => vi.advanceTimersByTime(2000));
    expect.soft(remaining()).toBe(before);
    expect.soft(view.commits.mock.calls.length).toBe(commits);
    expect.soft(approvalFocusAttempts()).toHaveLength(0);
    expect(document.activeElement).toBe(view.external);
  });

  it('freezes an existing hidden countdown and restores from the real deadline without taking external focus', async () => {
    const view = mount();
    await startGeneration();
    arrive();
    view.activity(false, true);
    view.external.focus();
    focus.mockClear();
    const countdown = screen.getByText(/剩余 \d+ 秒/);
    const before = countdown.textContent;
    const commits = view.commits.mock.calls.length;
    act(() => vi.advanceTimersByTime(8000));
    expect.soft(countdown.textContent).toBe(before);
    expect.soft(view.commits.mock.calls.length).toBe(commits);
    view.activity(true);
    expect(screen.getByText('提案已过期，无法再批准。')).toBeTruthy();
    expect((screen.getByRole('button', { name: /批准本次调用/ }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByRole('button', { name: /拒绝并停止/ }) as HTMLButtonElement).disabled).toBe(true);
    expect(approvalFocusAttempts()).toHaveLength(0);
    expect(document.activeElement).toBe(view.external);
    const expiredCommits = view.commits.mock.calls.length;
    act(() => vi.advanceTimersByTime(5000));
    expect(view.commits.mock.calls.length).toBe(expiredCommits);
  });

  it('restores a still-valid covered approval using its original deadline and never refocuses it', async () => {
    const view = mount(false);
    await startGeneration();
    arrive(proposal('covered', 10));
    act(() => vi.advanceTimersByTime(3000));
    focus.mockClear();
    view.external.focus();
    view.activity(true);
    expect(remaining()).toContain('7');
    expect(approvalFocusAttempts()).toHaveLength(0);
    expect(document.activeElement).toBe(view.external);
    act(() => vi.advanceTimersByTime(1000));
    expect(remaining()).toContain('6');
  });

  it('stops presentation commits once an active approval expires', async () => {
    const view = mount();
    await startGeneration();
    arrive(proposal('short', 1));
    act(() => vi.advanceTimersByTime(1000));
    expect(screen.getByText('提案已过期，无法再批准。')).toBeTruthy();
    const commits = view.commits.mock.calls.length;
    act(() => vi.advanceTimersByTime(1000));
    act(() => vi.advanceTimersByTime(1000));
    expect(view.commits.mock.calls.length).toBe(commits);
  });

  it('disables decisions at a deadline between one-second refreshes', async () => {
    mount();
    await startGeneration();
    arrive(proposal('fractional', 1.25));
    act(() => vi.advanceTimersByTime(1000));
    expect(remaining()).toContain('1');
    act(() => vi.advanceTimersByTime(250));
    expect(screen.getByText('提案已过期，无法再批准。')).toBeTruthy();
    expect((screen.getByRole('button', { name: /批准本次调用/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it('does not focus an existing approval when returning to its internal session', async () => {
    const view = mount();
    await startGeneration();
    arrive();
    await selectSession(1);
    view.external.focus();
    focus.mockClear();
    await selectSession(0);
    expect(heading()).toBeTruthy();
    expect(approvalFocusAttempts()).toHaveLength(0);
    expect(document.activeElement).toBe(view.external);
    arrive(proposal('new-after-return'));
    expect(approvalFocusAttempts()).toHaveLength(1);
    expect(document.activeElement).toBe(heading());
  });

  it('receives a background-session approval without focusing it when that session is selected', async () => {
    const view = mount();
    await startGeneration();
    await selectSession(1);
    arrive();
    emit({ kind: 'delta', assistant_message_id: 'a1', text: '其他会话流文本' });
    expect(useChatStore.getState().generation).toBeNull();
    expect(useChatStore.getState().generations.s1.text).toBe('其他会话流文本');
    view.external.focus();
    focus.mockClear();
    await selectSession(0);
    expect(heading()).toBeTruthy();
    expect(screen.getByText('其他会话流文本')).toBeTruthy();
    expect(approvalFocusAttempts()).toHaveLength(0);
    expect(document.activeElement).toBe(view.external);
  });
});

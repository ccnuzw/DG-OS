import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ModalHost } from '@dgos/ui';
import { invoke } from '@tauri-apps/api/core';
import ReplyExecution, { useReplyRun } from '../src/components/chat/ReplyExecution';
import RunInspector from '../src/components/chat/RunInspector';
import WebControls from '../src/components/chat/WebControls';
import { executionSteps, trustedSources } from '../src/components/chat/sources';
import { chatErrorText, ChatCommandError, createChatStore, useChatStore, type ChatChannel, type ChatRun, type ChatRunStep, type ChatSession } from '../src/stores/chatStore';
import { chatBudget } from './helpers/chatBudget';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), Channel: class { onmessage = () => {}; } }));
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl: vi.fn() }));
const ok = (data: unknown) => ({ success: true, data });
const session: ChatSession = { id: 's', title: 'Request', title_source: 'user', provider_id: 'p', model_id: 'm', skills: [], generation: {}, no_skill_confirmed: true, created_at: 1, updated_at: 1 };
const step: ChatRunStep = { id: 'step', ordinal: 1, kind: 'tool', phase: 'tool', status: 'settled', dispatched: true, result_unknown: false, receipt: { server_id: 'calendar', tool_name: 'list_events', outcome: 'success', result: { content: [{ type: 'text', text: 'Two events found' }] } }, created_at: 1, updated_at: 2 };
const run: ChatRun = { task_id: 'task', session_id: 's', assistant_message_id: 'reply', source_run_id: null, status: 'completed', partial_content: 'Final', dispatched: true, result_unknown: false, audit_warning: null, total_tokens: null, steps: [step], created_at: 1, updated_at: 2 };

function HistoricalReply({ taskId = 'task', messageId = 'reply' }: { taskId?: string; messageId?: string }) {
  const { row, projection } = useReplyRun('s', taskId, messageId);
  return <article ref={row} data-testid={taskId} aria-label={`Historical ${taskId}`}><span>{projection?.loaded ? 'loaded' : 'missing'}</span>{projection ? <ReplyExecution taskId={taskId} steps={projection.steps} run={projection.run} /> : null}</article>;
}

afterEach(() => { cleanup(); useChatStore.getState().reset(); vi.clearAllMocks(); vi.unstubAllGlobals(); });

describe('reply execution reading', () => {
  it('renders no empty execution card for a plain-text reply', () => {
    const view = render(<ReplyExecution taskId="task" steps={[]} run={{ ...run, steps: [] }} />);
    expect(view.container.childElementCount).toBe(0);
  });

  it('keeps a user expansion while live facts settle and shows human-readable text, not JSON', () => {
    const view = render(<ReplyExecution taskId="task" steps={[{ ...step, status: 'dispatched', result_unknown: true, receipt: null }]} />);
    const disclosure = screen.getByRole('button', { expanded: false });
    fireEvent.click(disclosure);
    expect(disclosure.getAttribute('aria-expanded')).toBe('true');
    view.rerender(<ReplyExecution taskId="task" steps={[step]} run={run} />);
    expect(screen.getByRole('button', { expanded: true })).toBeTruthy();
    expect(screen.getByText('Two events found')).toBeTruthy();
    expect(view.container.querySelector('pre')).toBeNull();
    expect(view.container.textContent).not.toContain('schema_hash');
  });

  it('coalesces stable web activity identities and accepts citations only from host web receipts', () => {
    const source = { id: 'host-source', url: 'https://evidence.example.invalid/', title: 'Evidence', excerpt: 'Actual snippet', retrieved_at: 1, kind: 'search_result', truncated: false };
    const running = { ...step, id: 'web-start', kind: 'web', receipt: { web_activity: { id: 'web-1', operation: 'search', status: 'running', query: 'question', sources: [], truncated: false } } };
    const completed = { ...running, id: 'web-finish', receipt: { web_activity: { ...running.receipt.web_activity, status: 'completed', sources: [source] } } };
    expect(executionSteps([running, completed])).toEqual([completed]);
    expect(trustedSources([running, completed])).toEqual([source]);
    expect(trustedSources([{ ...step, receipt: { result: { sources: [source] } } }])).toEqual([]);
    expect(trustedSources([{ ...completed, receipt: { web_activity: { ...completed.receipt.web_activity, sources: [{ ...source, url: 'javascript:alert(1)' }] } } }])).toEqual([]);
  });

  it('explains a rejected complex webpage without exposing internal reason codes as the result', () => {
    const failed: ChatRunStep = { ...step, kind: 'web', status: 'failed', receipt: { web_activity: { id: 'complex-page', operation: 'fetch', status: 'failed', url: 'https://evidence.example.invalid/', sources: [], error: 'web_html_complexity_limit', truncated: false } } };
    const view = render(<ReplyExecution taskId="task" steps={[failed]} run={{ ...run, steps: [failed] }} />);
    expect(view.container.textContent).not.toContain('web_html_complexity_limit');
    expect(view.container.textContent).toContain(chatErrorText(new ChatCommandError(1006022, { reason: 'web_html_complexity_limit' })));
    expect(view.container.querySelector('pre')).toBeNull();
  });
});

describe('ordered reply projections', () => {
  function fixture() {
    let channel: ChatChannel | undefined;
    const invokeCommand = vi.fn(async (command: string, args?: Record<string, unknown>) => {
      if (command === 'list_chat_messages') return ok({ items: [], next_before_seq: null });
      if (command === 'list_chat_sessions') return ok({ items: [session], next_cursor: null });
      if (command === 'inspect_chat_context') return ok(chatBudget({ model_id: session.model_id! }));
      if (command === 'get_chat_run') return ok({ ...run, status: 'running' });
      if (command === 'send_chat_message') {
        const input = args?.input as { client_request_id: string };
        return ok({ task_id: input.client_request_id, user_message_id: 'user', assistant_message_id: 'reply', dropped_turns: 0, replayed: false });
      }
      throw new Error(command);
    });
    const store = createChatStore({ invoke: invokeCommand as never, createChannel: () => { channel = { onmessage: () => {} }; return channel; }, newRequestId: () => 'task' });
    store.setState({ sessions: [session] });
    return { store, invokeCommand, emit: (event: Parameters<ChatChannel['onmessage']>[0]) => channel!.onmessage(event) };
  }

  it('deduplicates old events, segments model rounds and reconciles gaps without dispatching again', async () => {
    const f = fixture();
    await f.store.getState().selectSession(session);
    await f.store.getState().send('Question');
    f.emit({ kind: 'model_segment', seq: 1, task_id: 'task', assistant_message_id: 'reply', round: 1 });
    f.emit({ kind: 'delta', seq: 2, assistant_message_id: 'reply', text: 'Intermediate' });
    f.emit({ kind: 'delta', seq: 2, assistant_message_id: 'reply', text: 'duplicate' });
    f.emit({ kind: 'model_segment', seq: 3, task_id: 'task', assistant_message_id: 'reply', round: 2 });
    expect(f.store.getState().generation?.segments).toEqual([{ round: 1, content: 'Intermediate' }]);
    expect(f.store.getState().generation?.text).toBe('');
    f.emit({ kind: 'delta', seq: 5, assistant_message_id: 'reply', text: 'Final' });
    await waitFor(() => expect(f.invokeCommand.mock.calls.filter(([name]) => name === 'get_chat_run')).toHaveLength(1));
    expect(f.invokeCommand.mock.calls.filter(([name]) => name === 'send_chat_message')).toHaveLength(1);
    f.emit({ kind: 'done', seq: 6, message: { id: 'reply', session_id: 's', seq: 2, role: 'assistant', content: 'Persisted final', status: 'completed', task_id: 'task', finish_reason: 'stop', error_code: null, total_tokens: null, result_unknown: false, created_at: 1, updated_at: 2 } });
    f.emit({ kind: 'delta', seq: 7, assistant_message_id: 'reply', text: 'late' });
    expect(f.store.getState().generation).toBeNull();
    expect(f.store.getState().messages.at(-1)?.content).toBe('Persisted final');
  });

  it.each([false, true])('recovers a missing model boundary with concurrent facts (snapshot includes new facts: %s)', async includesNew => {
    const f = fixture();
    await f.store.getState().selectSession(session);
    await f.store.getState().send('Question');
    const pending = Promise.withResolvers<unknown>();
    const original = f.invokeCommand.getMockImplementation()!;
    f.invokeCommand.mockImplementation((command, args) => command === 'get_chat_run' ? pending.promise as never : original(command, args));
    f.emit({ kind: 'model_segment', seq: 1, task_id: 'task', assistant_message_id: 'reply', round: 1 });
    f.emit({ kind: 'delta', seq: 2, assistant_message_id: 'reply', text: 'First' });
    f.emit({ kind: 'delta', seq: 4, assistant_message_id: 'reply', text: 'Second' });
    f.emit({ kind: 'delta', seq: 5, assistant_message_id: 'reply', text: 'New' });
    const completed = { ...step, updated_at: 3 };
    f.emit({ kind: 'activity', seq: 6, task_id: 'task', assistant_message_id: 'reply', step: completed });
    const intermediate: ChatRunStep = { ...step, id: 'round-one', kind: 'model_segment', receipt: { round: 1, content: 'First' } };
    const prior = { ...step, status: 'dispatched', receipt: null };
    pending.resolve(ok({ ...run, status: 'running', partial_content: 'Lagging checkpoint, not a stream snapshot', steps: [intermediate, prior], live_snapshot: { seq: includesNew ? 6 : 4, round: 2, text: includesNew ? 'SecondNew' : 'Second', reasoning: '', phase: 'final_answer', proposal: null, activities: [intermediate, includesNew ? completed : prior], terminal: null } }));
    await waitFor(() => expect(f.store.getState().generation?.lastSeq).toBe(6));
    expect(f.store.getState().generation).toMatchObject({ text: 'SecondNew', round: 2, hostPhase: 'final_answer', activities: [intermediate, completed] });
    f.emit({ kind: 'delta', seq: 5, assistant_message_id: 'reply', text: 'duplicate' });
    f.emit({ kind: 'delta', seq: 7, assistant_message_id: 'reply', text: 'New' });
    expect(f.store.getState().generation?.text).toBe('SecondNewNew');
    expect(f.invokeCommand.mock.calls.filter(([name]) => name === 'get_chat_run')).toHaveLength(1);
    expect(f.invokeCommand.mock.calls.filter(([name]) => name === 'send_chat_message')).toHaveLength(1);
  });

  it('does not poll an unchanged watermark or replay events across another unresolved gap', async () => {
    const f = fixture();
    await f.store.getState().selectSession(session);
    await f.store.getState().send('Question');
    const original = f.invokeCommand.getMockImplementation()!;
    f.invokeCommand.mockImplementation((command, args) => command === 'get_chat_run' ? Promise.resolve(ok({ ...run, status: 'running', live_snapshot: { seq: 1, round: 1, text: '', reasoning: '', phase: null, proposal: null, activities: [], terminal: null } })) : original(command, args));
    f.emit({ kind: 'started', seq: 1, task_id: 'task', assistant_message_id: 'reply' });
    f.emit({ kind: 'delta', seq: 3, assistant_message_id: 'reply', text: 'Not continuous' });
    await waitFor(() => expect(f.invokeCommand.mock.calls.filter(([name]) => name === 'get_chat_run')).toHaveLength(1));
    await Promise.resolve();
    expect(f.store.getState().generation?.text).toBe('');
    expect(f.invokeCommand.mock.calls.filter(([name]) => name === 'get_chat_run')).toHaveLength(1);
    f.emit({ kind: 'done', seq: 4, message: { id: 'reply', session_id: 's', seq: 2, role: 'assistant', content: 'Authoritative final', status: 'completed', task_id: 'task', finish_reason: 'stop', error_code: null, total_tokens: null, result_unknown: false, created_at: 1, updated_at: 2 } });
    expect(f.store.getState().generation).toBeNull();
    expect(f.store.getState().messages.at(-1)?.content).toBe('Authoritative final');
  });

  it('settles a missed terminal from the live snapshot instead of replaying its buffered deltas', async () => {
    const f = fixture();
    await f.store.getState().selectSession(session);
    await f.store.getState().send('Question');
    const pending = Promise.withResolvers<unknown>();
    const original = f.invokeCommand.getMockImplementation()!;
    f.invokeCommand.mockImplementation((command, args) => command === 'get_chat_run' ? pending.promise as never : original(command, args));
    f.emit({ kind: 'started', seq: 1, task_id: 'task', assistant_message_id: 'reply' });
    f.emit({ kind: 'delta', seq: 3, assistant_message_id: 'reply', text: 'Buffered display text' });
    const message = { id: 'reply', session_id: 's', seq: 2, role: 'assistant' as const, content: 'Persisted terminal answer', status: 'completed' as const, task_id: 'task', finish_reason: 'stop', error_code: null, total_tokens: null, result_unknown: false, created_at: 1, updated_at: 2 };
    pending.resolve(ok({ ...run, live_snapshot: { seq: 4, round: 1, text: message.content, reasoning: '', phase: null, proposal: null, activities: [], terminal: { kind: 'done', seq: 4, message } } }));
    await waitFor(() => expect(f.store.getState().generation).toBeNull());
    expect(f.store.getState().messages.at(-1)?.content).toBe(message.content);
    expect(f.invokeCommand.mock.calls.filter(([name]) => name === 'send_chat_message')).toHaveLength(1);
  });

  it('rejects a snapshot bound to another reply and ignores delayed account responses', async () => {
    const f = fixture();
    await f.store.getState().selectSession(session);
    f.invokeCommand.mockImplementation(async () => ok({ ...run, assistant_message_id: 'someone-else' }));
    await f.store.getState().loadReplyRun('s', 'task', 'reply');
    expect(f.store.getState().replyRuns.task.unavailable).toBe(true);
    const pending = Promise.withResolvers<unknown>();
    f.invokeCommand.mockImplementation(() => pending.promise as never);
    const loading = f.store.getState().loadReplyRun('s', 'task', 'reply', true);
    f.store.getState().reset();
    pending.resolve(ok(run));
    await loading;
    expect(f.store.getState().replyRuns).toEqual({});
  });
  it('does not let a pre-verification read restore stale unknown evidence', async () => {
    const f = fixture();
    await f.store.getState().selectSession(session);
    const unknown: ChatRun = { ...run, result_unknown: true, steps: [{ ...step, result_unknown: true, status: 'dispatched', receipt: null }] };
    const updated: ChatRun = { ...run, steps: [{ ...step, status: 'manually_resolved', receipt: { source: 'user_report', outcome: 'completed', note: 'External receipt checked' } }] };
    const pending = Promise.withResolvers<unknown>();
    const original = f.invokeCommand.getMockImplementation()!;
    f.invokeCommand.mockImplementation((command, args) => {
      if (command === 'get_chat_run') return pending.promise as never;
      if (command === 'resolve_chat_run_step') return Promise.resolve(ok(updated));
      if (command === 'list_chat_runs') return Promise.resolve(ok({ items: [unknown] }));
      return original(command, args);
    });
    const reading = f.store.getState().loadReplyRun('s', 'task', 'reply');
    await f.store.getState().resolveRunStep('task', 'step', 'completed', 'External receipt checked');
    pending.resolve(ok(unknown));
    await reading;
    expect(f.store.getState().replyRuns.task.run).toEqual(updated);
    expect(f.store.getState().sessionUnknown.s).toBe(false);
  });

  it('bounds concurrent historical reads and coalesces repeated reads of one reply', async () => {
    const f = fixture();
    await f.store.getState().selectSession(session);
    const pending: PromiseWithResolvers<unknown>[] = [];
    f.invokeCommand.mockImplementation(() => { const request = Promise.withResolvers<unknown>(); pending.push(request); return request.promise as never; });
    const reads = ['a', 'b', 'c', 'd'].map(taskId => f.store.getState().loadReplyRun('s', taskId, 'reply'));
    const repeated = f.store.getState().loadReplyRun('s', 'a', 'reply');
    expect(pending).toHaveLength(2);
    pending[0].resolve(ok({ ...run, task_id: 'a' })); pending[1].resolve(ok({ ...run, task_id: 'b' }));
    await waitFor(() => expect(pending).toHaveLength(4));
    pending[2].resolve(ok({ ...run, task_id: 'c' })); pending[3].resolve(ok({ ...run, task_id: 'd' }));
    await Promise.all([...reads, repeated]);
    expect(Object.keys(f.store.getState().replyRuns)).toHaveLength(4);
    expect(pending).toHaveLength(4);
  });
});

describe('visible reply cache and recovery projections', () => {
  it('retains more than 24 visible replies without refetch loops and reloads an evicted reply on re-entry', async () => {
    const observers: { notify: (visible: boolean) => void }[] = [];
    vi.stubGlobal('IntersectionObserver', class {
      element: Element | null = null;
      constructor(private callback: IntersectionObserverCallback) { observers.push(this); }
      observe(element: Element) { this.element = element; }
      disconnect() { this.element = null; }
      notify(visible: boolean) { if (this.element) this.callback([{ target: this.element, isIntersecting: visible } as IntersectionObserverEntry], this as unknown as IntersectionObserver); }
    });
    useChatStore.setState({ sessions: [session], activeSessionId: 's', activeSession: session });
    vi.mocked(invoke).mockImplementation(async (_command, args) => {
      const input = args!.input as { task_id: string }; // This fixture is called only by loadReplyRun.
      const taskId = input.task_id;
      return ok({ ...run, task_id: taskId, assistant_message_id: taskId, steps: [] }) as never;
    });
    const ids = Array.from({ length: 30 }, (_, index) => `visible-${index}`);
    const view = render(<div className="chat-message-scroll">{ids.map(id => <HistoricalReply key={id} taskId={id} messageId={id} />)}</div>);
    act(() => observers.forEach(observer => observer.notify(true)));
    await waitFor(() => expect(ids.every(id => useChatStore.getState().replyRuns[id]?.loaded)).toBe(true));
    expect(vi.mocked(invoke).mock.calls).toHaveLength(30);
    expect(Object.keys(useChatStore.getState().replyRuns)).toHaveLength(30);
    act(() => observers[0].notify(false));
    expect(useChatStore.getState().replyRuns[ids[0]]).toBeUndefined();
    expect(within(screen.getByTestId(ids[0])).getByText('missing')).toBeTruthy();
    act(() => observers[0].notify(true));
    await waitFor(() => expect(within(screen.getByTestId(ids[0])).getByText('loaded')).toBeTruthy());
    await act(async () => {});
    expect(vi.mocked(invoke).mock.calls).toHaveLength(31);
    view.unmount();
    expect(Object.keys(useChatStore.getState().replyRuns)).toHaveLength(24);
  });

  it.each(['reply', 'inspector'])('publishes a %s verification to the mounted reply and shared uncertainty state', async entry => {
    vi.stubGlobal('IntersectionObserver', undefined);
    let observed: ChatRun = { ...run, status: 'interrupted', result_unknown: true, steps: [{ ...step, ordinal: 0, status: 'dispatched', result_unknown: true, receipt: null }] };
    useChatStore.setState({ sessions: [session], activeSessionId: 's', activeSession: session });
    vi.mocked(invoke).mockImplementation(async (command, args) => {
      if (command === 'get_chat_run') return ok(observed) as never;
      if (command === 'resolve_chat_run_step') {
        const input = args!.input as { outcome: string; note: string };
        observed = { ...observed, result_unknown: false, steps: [{ ...observed.steps[0], result_unknown: false, status: 'manually_resolved', receipt: { source: 'user_report', outcome: input.outcome, note: input.note } }] };
        return ok(observed) as never;
      }
      if (command === 'list_chat_runs') return ok({ items: [observed] }) as never;
      if (command === 'list_chat_messages') return ok({ items: [], next_before_seq: null }) as never;
      throw new Error(command);
    });
    await useChatStore.getState().loadReplyRun('s', 'task', 'reply');
    render(<><HistoricalReply /><RunInspector sessionId="s" taskId="task" messageId="reply" /><ModalHost /></>);
    const article = screen.getByRole('article', { name: 'Historical task' });
    expect((within(article).getByRole('button', { name: '用新请求继续' }) as HTMLButtonElement).disabled).toBe(true);
    let controls: HTMLElement = article;
    if (entry === 'inspector') {
      fireEvent.click(screen.getByRole('button', { name: '诊断详情' }));
      controls = await screen.findByRole('dialog', { name: '诊断详情' });
      await within(controls).findByText('记录第 1 项操作的核查结果');
    }
    fireEvent.click(within(controls).getByText('记录第 1 项操作的核查结果'));
    fireEvent.click(within(controls).getByRole('button', { name: '我已核查：操作已完成' }));
    fireEvent.change(within(controls).getByRole('textbox', { name: '你如何核查？请附上结果或参考记录。' }), { target: { value: 'Checked the unique external receipt' } });
    fireEvent.click(within(controls).getByRole('checkbox', { name: '我已在此回复之外核查实际结果，并理解这只是我的报告，不是服务出具的权威回执。' }));
    fireEvent.click(within(controls).getByRole('button', { name: '保存核查结果' }));
    await waitFor(() => expect(useChatStore.getState().replyRuns.task.run?.steps[0].result_unknown).toBe(false));
    await waitFor(() => expect((within(article).getByRole('button', { name: '用新请求继续', hidden: true }) as HTMLButtonElement).disabled).toBe(false));
    expect(useChatStore.getState().sessionUnknown.s).toBe(false);
    expect(article.textContent).not.toContain('操作可能已执行，结果尚未确认');
    expect(article.querySelector('.chat-activity-resolution')).toBeNull();
    expect(vi.mocked(invoke).mock.calls.filter(([command]) => command === 'resolve_chat_run_step')).toHaveLength(1);
    expect(vi.mocked(invoke).mock.calls.some(([command]) => ['send_chat_message', 'continue_chat_run', 'advance_chat_tool_turn'].includes(command))).toBe(false);
  });
});

describe('request-scoped web controls', () => {
  it('allows fetch without a search credential and scrubs unsaved key on close', async () => {
    useChatStore.setState({ activeSessionId: 's', activeSession: session, sessions: [session] });
    vi.mocked(invoke).mockResolvedValue(ok({ backend: 'brave', search_configured: false, fetch_available: true }));
    render(<><WebControls sessionId="s" disabled={false} /><ModalHost /></>);
    fireEvent.click(screen.getByRole('button', { name: '联网查询已关闭' }));
    const fetch = await screen.findByRole('checkbox', { name: '允许读取公开网页' });
    await waitFor(() => expect((fetch as HTMLButtonElement).disabled).toBe(false));
    expect((screen.getByRole('checkbox', { name: '允许通过 Brave Search 搜索' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(fetch);
    expect(useChatStore.getState().sessionWebAccess.s).toEqual({ search: false, fetch: true });
    fireEvent.click(screen.getByRole('button', { name: '配置 Brave Search' }));
    const field = screen.getByLabelText('Brave Search API Key');
    fireEvent.change(field, { target: { value: 'synthetic-secret' } });
    expect(document.body.textContent).not.toContain('synthetic-secret');
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    fireEvent.click(screen.getByRole('button', { name: '已为下一次请求启用联网查询' }));
    fireEvent.click(await screen.findByRole('button', { name: '配置 Brave Search' }));
    expect((screen.getByLabelText('Brave Search API Key') as HTMLInputElement).value).toBe('');
    expect(vi.mocked(invoke).mock.calls.some(([command]) => command === 'configure_chat_web')).toBe(false);
  });

  it('sends the explicit request grant and resets it after request admission', async () => {
    let channel: ChatChannel | undefined;
    const command = vi.fn(async (name: string, _args?: Record<string, unknown>) => {
      if (name === 'list_chat_messages') return ok({ items: [], next_before_seq: null });
      if (name === 'inspect_chat_context') return ok(chatBudget({ model_id: session.model_id! }));
      if (name === 'send_chat_message') return ok({ task_id: 'task', user_message_id: 'user', assistant_message_id: 'reply', dropped_turns: 0, replayed: false });
      return ok({ items: [session], next_cursor: null });
    });
    const store = createChatStore({ invoke: command as never, createChannel: () => { channel = { onmessage: () => {} }; return channel; }, newRequestId: () => 'task' });
    store.setState({ sessions: [session] });
    await store.getState().selectSession(session);
    store.getState().setWebAccess('s', { search: false, fetch: true });
    await store.getState().send('Read this URL');
    const request = command.mock.calls.find(([name]) => name === 'send_chat_message');
    expect((request?.[1]?.input as Record<string, unknown>).web_access).toEqual({ search: false, fetch: true });
    expect(store.getState().sessionWebAccess.s).toEqual({ search: false, fetch: false });
    expect(channel).toBeDefined();
  });

  it('does not classify a web admission failure as an invalid tool proposal', async () => {
    const command = vi.fn(async (name: string) => {
      if (name === 'list_chat_messages') return ok({ items: [], next_before_seq: null });
      if (name === 'inspect_chat_context') return ok(chatBudget({ model_id: session.model_id! }));
      if (name === 'send_chat_message') return { success: false, error: { code: 1006022, detail: { reason: 'web_search_key_required' } } };
      return ok({ items: [session], next_cursor: null });
    });
    const store = createChatStore({ invoke: command as never, createChannel: () => ({ onmessage: () => {} }), newRequestId: () => 'task' });
    store.setState({ sessions: [session] });
    await store.getState().selectSession(session);
    store.getState().setDraft('s', 'Search this question');
    expect(await store.getState().send('Search this question')).toBe('rejected');
    expect(store.getState().sessionActionCodes.s).toBeNull();
    expect(store.getState().sessionActionErrors.s).toBe(chatErrorText(new ChatCommandError(1006022, { reason: 'web_search_key_required' })));
    expect(store.getState().sessionDrafts.s).toBe('Search this question');
  });
});

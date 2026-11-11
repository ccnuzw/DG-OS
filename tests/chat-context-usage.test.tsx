import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import ContextPanel from '../src/components/chat/ContextPanel';
import { compactTokens, compareContextBudgets, contextUsage, usageFieldPartial } from '../src/components/chat/contextUsage';
import { createChatStore, useChatStore, type ChatMessage, type ChatRun, type ChatSession, type ChatSummary } from '../src/stores/chatStore';
import { setLocaleForTesting } from '../src/i18n';
import { chatBudget } from './helpers/chatBudget';
import { chatUsage } from './helpers/chatUsage';
import { MockChannel } from './helpers/mockChannel';

// vi.mock is hoisted before static imports; the factory must load its Channel dependency inside the mock boundary.
vi.mock('@tauri-apps/api/core', async () => {
  const { MockChannel: Channel } = await import('./helpers/mockChannel');
  return { invoke: vi.fn(), Channel };
});
const ok = (data: unknown) => ({ success: true, data });
const session: ChatSession = { id: 's1', title: 'Usage regression', title_source: 'user', provider_id: 'p1', model_id: 'chat1', skills: [], generation: {}, no_skill_confirmed: true, created_at: 1, updated_at: 1 };
const summary: ChatSummary = { id: 'installed-summary', source_first_seq: 1, source_last_seq: 4, source_digest: 'synthetic-digest', content: 'Synthetic retained summary', provider_id: 'p1', model_id: 'chat1', created_at: 2 };
const mocked = vi.mocked(invoke);
let budgetHandler: () => unknown;
let usageHandler: () => unknown;
const calls = (name: string) => mocked.mock.calls.filter(([command]) => command === name);

beforeEach(() => {
  setLocaleForTesting('zh-CN');
  mocked.mockReset();
  MockChannel.reset();
  useChatStore.getState().reset();
  useChatStore.setState({ sessions: [session], activeSessionId: session.id, activeSession: session, sessionViews: { s1: { messages: [], stale: false, loaded: true, olderCursor: null, newerTrimmed: false, messagesLoading: false, messagesError: null, droppedTurns: {} } } });
  budgetHandler = () => ok(chatBudget({ estimated_input_tokens: 4096, reserved_output_tokens: 1024, model_window_tokens: 10240 }));
  usageHandler = () => ok(chatUsage());
  mocked.mockImplementation(async (command, args) => {
    if (command === 'inspect_chat_context') return budgetHandler() as never;
    if (command === 'get_chat_usage') return usageHandler() as never;
    if (command === 'summarize_chat_context') {
      const input = args!.input as { client_request_id: string };
      return ok({ task_id: input.client_request_id, assistant_message_id: 'summary-reply', user_message_id: '', dropped_turns: 0, replayed: false }) as never;
    }
    if (command === 'list_chat_sessions') return ok({ items: [session], next_cursor: null }) as never;
    if (command === 'list_chat_runs') return ok({ items: [] }) as never;
    throw new Error(`Unexpected fixture command: ${command}`);
  });
});
afterEach(() => { cleanup(); useChatStore.getState().reset(); });

async function openUsage() {
  if (!screen.queryByRole('dialog', { name: 'Token 用量' })) fireEvent.click(screen.getByRole('button', { name: /^预计上下文：/ }));
  return screen.findByRole('dialog', { name: 'Token 用量' });
}
async function readyPanel() {
  const view = render(<ContextPanel session={session} content="draft" toolKeys={[]} disabled={false} />);
  await waitFor(() => expect(screen.getByRole('button', { name: /^预计上下文：/ }).getAttribute('aria-label')).toContain('50%'));
  return { ...view, panel: await openUsage() };
}
async function startSummary() {
  const panel = await openUsage();
  await waitFor(() => expect((within(panel).getByRole('button', { name: '压缩', exact: true }) as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(within(panel).getByRole('button', { name: '压缩', exact: true }));
  expect(calls('summarize_chat_context')).toHaveLength(0);
  const confirmation = await screen.findByRole('dialog', { name: '压缩较早对话？' });
  expect(confirmation.textContent).toContain('可能产生费用');
  fireEvent.click(within(confirmation).getByRole('button', { name: '确认压缩' }));
  await waitFor(() => expect(useChatStore.getState().generation?.kind).toBe('summary'));
}
function installSummary() {
  const generation = useChatStore.getState().generation!;
  act(() => MockChannel.last().emit({ kind: 'summary_done', task_id: generation.taskId, assistant_message_id: 'summary-reply', summary, dispatched: true, result_unknown: false, audit_warning: null }));
}

describe('next-request and durable usage projection', () => {
  it('counts images once and does not hide the independent host cap', () => {
    expect(contextUsage(chatBudget({ estimated_input_tokens: 4096, reserved_output_tokens: 1024, image_reserve_tokens: 2048, model_window_tokens: 10240 }))).toEqual({ requested: 5120, window: 10240, percent: 50, nearLimit: false });
    expect(contextUsage(chatBudget({ model_window_tokens: null, text_bytes: 20000 }))).toMatchObject({ window: null, percent: null, nearLimit: true });
    expect(contextUsage(chatBudget({ estimated_input_tokens: 7999, reserved_output_tokens: 0, model_window_tokens: 10000 }))).toMatchObject({ nearLimit: false });
  });
  it('compacts capacity only and reports field-level partial coverage without filling missing values', () => {
    expect([compactTokens(0), compactTokens(1536), compactTokens(2_000_000)]).toEqual(['0', '1.5k', '2M']);
    expect(usageFieldPartial(chatUsage({ coverage: { ...chatUsage().coverage, input: 1 } }), 'input')).toBe(true);
    expect(usageFieldPartial(chatUsage(), 'input')).toBe(false);
    expect(usageFieldPartial(chatUsage({ legacy: true }), 'input')).toBe(true);
    expect(usageFieldPartial(chatUsage(), 'cache_write')).toBe(false);
  });
  it('compares only unchanged nonhistory inputs and never fabricates a percentage for unknown windows', () => {
    const before = chatBudget({ model_window_tokens: null, estimated_input_tokens: 5000 });
    const after = chatBudget({ model_window_tokens: null, estimated_input_tokens: 3000 });
    expect(compareContextBudgets(before, after, true)).toEqual({ before: 6024, after: 4024, unit: 'tokens', lowered: true });
    expect(compareContextBudgets(before, after, false)).toBeNull();
    expect(compareContextBudgets(before, { ...after, model_id: 'other-model' }, true)).toBeNull();
    expect(compareContextBudgets(before, before, true)?.lowered).toBe(false);
  });
  it('uses the frozen camelCase session command and accepts an explicit reported zero', async () => {
    usageHandler = () => ok(chatUsage({ total_tokens: 0, input_tokens: 0, output_tokens: 0 }));
    expect((await useChatStore.getState().inspectUsage('s1'))?.total_tokens).toBe(0);
    expect(calls('get_chat_usage')[0][1]).toEqual({ sessionId: 's1' });
  });
  it.each(['account', 'session', 'model', 'newer-read'])('rejects late usage after a changed %s', async change => {
    const pending = Promise.withResolvers<unknown>();
    usageHandler = () => pending.promise;
    const reading = useChatStore.getState().inspectUsage('s1');
    if (change === 'account') useChatStore.getState().reset();
    if (change === 'session') useChatStore.setState({ activeSessionId: 's2' });
    if (change === 'model') useChatStore.setState({ sessions: [{ ...session, model_id: 'other-model' }] });
    if (change === 'newer-read') {
      usageHandler = () => ok(chatUsage({ total_tokens: 42 }));
      expect((await useChatStore.getState().inspectUsage('s1'))?.total_tokens).toBe(42);
    }
    pending.resolve(ok(chatUsage()));
    expect(await reading).toBeNull();
  });
  it('rejects an old target context response at the store boundary', async () => {
    const pending = Promise.withResolvers<unknown>();
    const store = createChatStore({ invoke: async () => pending.promise as never });
    store.setState({ sessions: [session], activeSessionId: 's1', activeSession: session });
    const reading = store.getState().inspectContext('s1', 'draft');
    store.setState({ sessions: [{ ...session, model_id: 'changed-model' }] });
    pending.resolve(ok(chatBudget()));
    expect(await reading).toBeNull();
  });
});

describe('compact usage panel regressions', () => {
  it('shows the newest settled unknown in details rather than recycling an older known total', async () => {
    usageHandler = () => ok(chatUsage({ latest: { model_id: 'latest-unknown-model', total_tokens: null, input_tokens: null, output_tokens: null, cache_read_tokens: null, duration_ms: null, created_at: null, partial: true } }));
    const { panel } = await readyPanel();
    fireEvent.click(within(panel).getByRole('button', { name: '查看详情' }));
    const latest = panel.querySelector('.chat-usage-latest')!;
    expect(latest.textContent).toContain('latest-unknown-model');
    expect(latest.textContent).toContain('部分已回报');
    expect(latest.textContent).not.toContain('1970');
    expect(Array.from(latest.querySelectorAll('dd'), node => node.textContent)).toEqual(['—', '—', '—', '—', '—']);
  });
  it('reports an installed summary whose estimate did not decrease without promising savings', async () => {
    await readyPanel();
    await startSummary();
    budgetHandler = () => ok(chatBudget({ estimated_input_tokens: 4096, reserved_output_tokens: 1024, model_window_tokens: 10240, summary }));
    usageHandler = () => ok(chatUsage({ total_tokens: 2000 }));
    installSummary();
    await openUsage();
    await screen.findByText('压缩完成，预估占用未降低');
    expect(calls('summarize_chat_context')).toHaveLength(1);
  });
  it('does not use loaded messages as a fallback when the durable session query fails', async () => {
    usageHandler = () => { throw new Error('durable usage unavailable'); };
    const message = { id: 'old-reply', session_id: 's1', seq: 1, role: 'assistant' as const, content: 'Loaded reply', status: 'completed' as const, task_id: null, finish_reason: 'stop', error_code: null, total_tokens: 999999, result_unknown: false, created_at: 1, updated_at: 1 };
    const state = useChatStore.getState();
    useChatStore.setState({ messages: [message], sessionViews: { s1: { ...state.sessionViews.s1, messages: [message] } } });
    render(<ContextPanel session={session} content="draft" toolKeys={[]} disabled={false} />);
    const panel = await openUsage();
    await within(panel).findByText('会话用量暂不可用');
    expect(within(panel).getByText('总计').closest('div')?.querySelector('dd')?.textContent).toBe('—');
    expect(panel.textContent).not.toContain('999,999');
  });
  it('renders only a ring trigger and six durable stats, with same-popover details, collapsed technical data and focus return', async () => {
    const { panel } = await readyPanel();
    const trigger = screen.getByRole('button', { name: /^预计上下文：/ });
    expect(trigger.textContent).toBe('');
    expect(panel.querySelectorAll('.chat-usage-stats > div')).toHaveLength(6);
    expect(within(panel).getByText('1,600')).toBeTruthy();
    expect(within(panel).queryByText('技术详情')).toBeNull();
    const details = within(panel).getByRole('button', { name: '查看详情' });
    fireEvent.click(details);
    const back = within(panel).getByRole('button', { name: '返回用量' });
    expect(document.activeElement).toBe(back);
    const technical = panel.querySelector<HTMLDetailsElement>('.chat-usage-debug')!;
    expect(technical.open).toBe(false);
    expect(panel.querySelector('.chat-usage-latest')?.textContent).toContain('chat1');
    expect(panel.querySelector('.chat-usage-latest')?.textContent).toContain('800');
    fireEvent.click(back);
    expect(document.activeElement).toBe(within(panel).getByRole('button', { name: '查看详情' }));
    fireEvent.keyDown(panel, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Token 用量' })).toBeNull());
    expect(document.activeElement).toBe(trigger);
  });
  it('leaves an unknown window unfilled while allowing host-approved compression with no loaded history', async () => {
    budgetHandler = () => ok(chatBudget({ model_window_tokens: null }));
    render(<ContextPanel session={session} content="draft" toolKeys={[]} disabled={false} />);
    const panel = await openUsage();
    await waitFor(() => expect((within(panel).getByRole('button', { name: '压缩', exact: true }) as HTMLButtonElement).disabled).toBe(false));
    expect(screen.getByRole('button', { name: /^预计上下文：/ }).getAttribute('aria-label')).toContain('容量未知');
    expect(panel.querySelector('progress')).toBeNull();
    expect(document.querySelectorAll('.chat-usage-ring circle')).toHaveLength(1);
    expect(useChatStore.getState().messages).toEqual([]);
  });
  it.each(['no_history', 'busy', 'unresolved', 'model_unavailable', 'context_too_long'])('respects host compression eligibility: %s', async reason => {
    budgetHandler = () => ok(chatBudget({ can_summarize: false, summary_unavailable_reason: reason }));
    render(<ContextPanel session={session} content="draft" toolKeys={[]} disabled={false} />);
    const panel = await openUsage();
    await waitFor(() => expect(calls('inspect_chat_context')).toHaveLength(1));
    await waitFor(() => expect(panel.querySelector('#chat-summary-reason')).not.toBeNull());
    expect((within(panel).getByRole('button', { name: '压缩', exact: true }) as HTMLButtonElement).disabled).toBe(true);
    expect(calls('summarize_chat_context')).toHaveLength(0);
  });
  it('rechecks an unresolved budget after verified evidence clears the shared unknown state', async () => {
    let observed: ChatRun = { task_id: 'unknown-run', session_id: 's1', assistant_message_id: 'old-reply', source_run_id: null, status: 'interrupted', partial_content: 'Retained reply', dispatched: true, result_unknown: true, audit_warning: null, total_tokens: null, steps: [{ id: 'unknown-step', ordinal: 1, kind: 'tool', phase: 'tool', status: 'dispatched', dispatched: true, result_unknown: true, receipt: null, created_at: 1, updated_at: 2 }], created_at: 1, updated_at: 2 };
    const original = mocked.getMockImplementation()!;
    mocked.mockImplementation(async (command, args) => {
      if (command === 'get_chat_run') return ok(observed) as never;
      if (command === 'resolve_chat_run_step') {
        observed = { ...observed, result_unknown: false, steps: [{ ...observed.steps[0], status: 'manually_resolved', result_unknown: false, receipt: { source: 'user_report', outcome: 'completed', note: 'External receipt checked' } }] };
        return ok(observed) as never;
      }
      if (command === 'list_chat_runs') return ok({ items: [observed] }) as never;
      if (command === 'list_chat_messages') return ok({ items: [], next_before_seq: null }) as never;
      return original(command, args);
    });
    await useChatStore.getState().inspectRun('unknown-run');
    budgetHandler = () => ok(chatBudget({ estimated_input_tokens: 4096, reserved_output_tokens: 1024, model_window_tokens: 10240, can_summarize: false, summary_unavailable_reason: 'unresolved' }));
    const { panel } = await readyPanel();
    expect((within(panel).getByRole('button', { name: '压缩', exact: true }) as HTMLButtonElement).disabled).toBe(true);
    budgetHandler = () => ok(chatBudget({ estimated_input_tokens: 4096, reserved_output_tokens: 1024, model_window_tokens: 10240 }));
    await act(async () => { await useChatStore.getState().resolveRunStep('unknown-run', 'unknown-step', 'completed', 'External receipt checked'); });
    expect(useChatStore.getState().sessionUnknown.s1).toBe(false);
    expect((within(panel).getByRole('button', { name: '压缩', exact: true }) as HTMLButtonElement).disabled).toBe(true);
    await waitFor(() => expect((within(panel).getByRole('button', { name: '压缩', exact: true }) as HTMLButtonElement).disabled).toBe(false));
    expect(calls('inspect_chat_context')).toHaveLength(2);
    expect(calls('summarize_chat_context')).toHaveLength(0);
  });

  it('refreshes changed history without confusing it with changed compression inputs', async () => {
    await readyPanel();
    await startSummary();
    const message: ChatMessage = { id: 'reloaded-history', session_id: 's1', seq: 2, role: 'assistant', content: 'History refreshed from the host', status: 'completed', task_id: 'past-run', finish_reason: 'stop', error_code: null, total_tokens: null, result_unknown: false, created_at: 1, updated_at: 3 };
    budgetHandler = () => ok(chatBudget({ estimated_input_tokens: 1024, reserved_output_tokens: 1024, model_window_tokens: 10240, summary }));
    act(() => useChatStore.setState(state => ({ messages: [message], sessionViews: { ...state.sessionViews, s1: { ...state.sessionViews.s1, messages: [message] } } })));
    installSummary();
    await openUsage();
    await screen.findByText('压缩完成：50% → 20%');
    expect(calls('summarize_chat_context')).toHaveLength(1);
  });

  it('invalidates a budget when loaded history changes without a generation or usage revision', async () => {
    await readyPanel();
    budgetHandler = () => ok(chatBudget({ estimated_input_tokens: 2048, reserved_output_tokens: 1024, model_window_tokens: 10240 }));
    const message: ChatMessage = { id: 'refreshed-reply', session_id: 's1', seq: 2, role: 'assistant', content: 'Changed retained history', status: 'completed', task_id: null, finish_reason: 'stop', error_code: null, total_tokens: null, result_unknown: false, created_at: 1, updated_at: 2 };
    act(() => useChatStore.setState(state => ({ messages: [message], sessionViews: { ...state.sessionViews, s1: { ...state.sessionViews.s1, messages: [message] } } })));
    expect(screen.getByText('更新中 · 上次预估')).toBeTruthy();
    await waitFor(() => expect(screen.getByRole('button', { name: /^预计上下文：/ }).getAttribute('aria-label')).toContain('30%'));
    expect(calls('inspect_chat_context')).toHaveLength(2);
  });
  it('keeps the same-target previous estimate marked updating and rejects stale draft responses', async () => {
    const view = await readyPanel();
    const pending = Promise.withResolvers<unknown>();
    budgetHandler = () => pending.promise;
    view.rerender(<ContextPanel session={session} content="changed draft" toolKeys={[]} disabled={false} />);
    expect(screen.getByText('更新中 · 上次预估')).toBeTruthy();
    expect(screen.getByRole('button', { name: /^预计上下文：/ }).getAttribute('aria-label')).toContain('50%');
    await waitFor(() => expect(calls('inspect_chat_context')).toHaveLength(2));
    budgetHandler = () => ok(chatBudget({ estimated_input_tokens: 1536, model_window_tokens: 10240 }));
    view.rerender(<ContextPanel session={session} content="newest draft" toolKeys={[]} disabled={false} />);
    await waitFor(() => expect(screen.getByRole('button', { name: /^预计上下文：/ }).getAttribute('aria-label')).toContain('25%'));
    await act(async () => pending.resolve(ok(chatBudget({ estimated_input_tokens: 9000, model_window_tokens: 10240 }))));
    expect(screen.getByRole('button', { name: /^预计上下文：/ }).getAttribute('aria-label')).toContain('25%');
  });
  it('invalidates model snapshots immediately and never substitutes loaded-message totals when the durable query fails', async () => {
    const view = await readyPanel();
    usageHandler = () => { throw new Error('usage unavailable'); };
    const target = { ...session, model_id: 'changed-model' };
    budgetHandler = () => ok(chatBudget({ model_id: target.model_id }));
    act(() => useChatStore.setState({ sessions: [target], activeSession: target }));
    view.rerender(<ContextPanel session={target} content="draft" toolKeys={[]} disabled={false} />);
    expect(screen.getByRole('button', { name: /^预计上下文：/ }).getAttribute('aria-label')).toContain('容量未知');
    const panel = await openUsage();
    await screen.findByText('会话用量暂不可用');
    expect(panel.textContent).not.toContain('已加载');
    expect(calls('get_chat_usage').length).toBeGreaterThan(1);
  });
  it('marks mixed cache coverage as partial rather than unsupported or a fabricated hit rate', async () => {
    usageHandler = () => ok(chatUsage({ cache_read_tokens: null, cache_hit_rate: null, partial: true, coverage: { ...chatUsage().coverage, cache_read: 1 } }));
    const { panel } = await readyPanel();
    expect(within(panel).getByText('其中缓存读取').closest('div')?.querySelector('dd')?.textContent).toBe('—部分');
    expect(within(panel).getByText('缓存命中率').closest('div')?.querySelector('dd')?.textContent).toBe('—');
  });
  it.each([0, null])('keeps reported total %s distinct from missing usage', async total => {
    usageHandler = () => ok(chatUsage({ total_tokens: total, input_tokens: total, output_tokens: total, cache_read_tokens: total, cache_hit_rate: null, partial: total === null }));
    const { panel } = await readyPanel();
    const totalRow = within(panel).getByText('总计').closest('div')!;
    expect(totalRow.querySelector('dd')?.textContent).toBe(total === null ? '—' : '0');
    expect(within(panel).getByText('缓存命中率').closest('div')?.querySelector('dd')?.textContent).toBe('—');
    if (total === null) expect(within(panel).getByText('部分已回报')).toBeTruthy();
  });
  it.each(['none', 'context', 'usage'])('retains compression progress across reopen and independently refreshes both sources (%s refresh failure)', async failure => {
    await readyPanel();
    await startSummary();
    const panel = await openUsage();
    expect(within(panel).getByRole('button', { name: '压缩中…' })).toBeTruthy();
    fireEvent.keyDown(panel, { key: 'Escape' });
    const reopened = await openUsage();
    expect((within(reopened).getByRole('button', { name: '压缩中…' }) as HTMLButtonElement).disabled).toBe(true);
    const beforeBudget = calls('inspect_chat_context').length;
    const beforeUsage = calls('get_chat_usage').length;
    budgetHandler = () => { if (failure === 'context') throw new Error('budget failed'); return ok(chatBudget({ estimated_input_tokens: 1024, reserved_output_tokens: 1024, model_window_tokens: 10240, summary })); };
    usageHandler = () => { if (failure === 'usage') throw new Error('usage failed'); return ok(chatUsage({ total_tokens: 2000 })); };
    installSummary();
    await waitFor(() => expect(calls('inspect_chat_context').length).toBeGreaterThan(beforeBudget));
    await waitFor(() => expect(calls('get_chat_usage').length).toBeGreaterThan(beforeUsage));
    if (failure === 'none') await screen.findByText('压缩完成：50% → 20%');
    else await screen.findByText('压缩完成，用量待更新');
    expect(useChatStore.getState().sessionSummaries.s1.id).toBe(summary.id);
    expect(calls('summarize_chat_context')).toHaveLength(1);
  });
  it('omits before/after claims when the draft changes during a paid compression', async () => {
    const view = await readyPanel();
    await startSummary();
    view.rerender(<ContextPanel session={session} content="different draft" toolKeys={[]} disabled={false} />);
    budgetHandler = () => ok(chatBudget({ estimated_input_tokens: 256, summary }));
    installSummary();
    await openUsage();
    await screen.findByText('压缩完成');
    expect(screen.queryByText(/压缩完成：/)).toBeNull();
  });
  it.each([false, true])('keeps a failed or unknown compression uninstalled and never retries automatically (unknown=%s)', async unknown => {
    await readyPanel();
    await startSummary();
    act(() => MockChannel.last().emit({ kind: 'error', assistant_message_id: 'summary-reply', message: null, error_code: 'provider_error', provider_code: null, result_unknown: unknown, regeneration_discarded: false, dispatched: true, audit_warning: null }));
    await openUsage();
    await screen.findByText(unknown ? /压缩结果未知/ : /压缩未完成/);
    expect(useChatStore.getState().sessionSummaries.s1).toBeUndefined();
    expect(calls('summarize_chat_context')).toHaveLength(1);
  });
});

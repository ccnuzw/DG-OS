import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { agentContextUsage } from '../src/components/development/agentContextUsage';
import { AgentContextDetails } from '../src/components/development/AgentContextDetails';
import { useAgentRunner, type AgentSession, type Json } from '../src/stores/agentRunnerStore';
import { useDevelopment } from '../src/stores/developmentStore';
import { codexOperationsKey, emptyCodexOperations, useCodexOperations } from '../src/stores/codexOperations';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }) }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); useCodexOperations.getState().reset(); invoke.mockReset(); });
const session = (engine: AgentSession['engine'] = 'claude'): AgentSession => ({ id: 'session', generation: 'generation', engine, native_id: 'native', workspace: '/workspace', status: 'idle', pending: [], error: null, events: [] });
const model = { inputTokens: 20, outputTokens: 5, cacheReadInputTokens: 100, cacheCreationInputTokens: 10, costUSD: 0.2, costBasis: 'list' };
function result(value: AgentSession, seq: number, run: string, data: Json) { value.events.push({ seq, run, kind: 'result', data, text: null }); }

it('uses Codex native totals without adding cache or reasoning subsets', () => {
  const value = session('codex');
  value.events.push({ seq: 1, kind: 'thread/tokenUsage/updated', text: null, data: { tokenUsage: { last: { inputTokens: 100, cachedInputTokens: 80, outputTokens: 20, reasoningOutputTokens: 10 }, total: { inputTokens: 500, cachedInputTokens: 400, outputTokens: 50, reasoningOutputTokens: 20 }, modelContextWindow: 1000 } } });
  expect(agentContextUsage(value)).toMatchObject({ used: 120, limit: 1000, cumulative: { total: 550, input: 500, output: 50, cached: 400 }, cacheHit: 0.8, costUsd: null, durationMs: null });
});

it('reads the latest cumulative Claude model receipt and never sums duplicate or prior results', () => {
  const value = session();
  result(value, 1, 'first', { model_usage: { main: model }, total_cost_usd: 0.2, duration_ms: 20 });
  result(value, 2, 'second', { model_usage: { main: { ...model, inputTokens: 40, costUSD: 0.3 } }, total_cost_usd: 0.3, duration_ms: 30 });
  result(value, 3, 'second', { model_usage: { main: { ...model, inputTokens: 40, costUSD: 0.3 } }, total_cost_usd: 0.3, duration_ms: 30 });
  expect(agentContextUsage(value)).toMatchObject({ cumulative: { input: 150, output: 5, cached: 100, total: 155 }, costUsd: 0.3, costEstimated: true, durationMs: 50 });
  // /clear may reset native cumulative figures. Never retain the old high water mark.
  result(value, 4, 'third', { model_usage: { main: { ...model, inputTokens: 1, costUSD: 0.01 } }, total_cost_usd: 0.01, duration_ms: 1 });
  expect(agentContextUsage(value).costUsd).toBe(0.01);
});

it('uses result usage as final turn accounting while block input alone estimates context', () => {
  const value = session();
  for (const [seq, output] of [[1, 900], [2, 1800]]) value.events.push({ seq, kind: 'message', run: 'turn', text: null, data: { type: 'assistant', native_message_id: 'api', model: 'main', usage_source: 'native_response_input', usage_final: false, usage: { input_tokens: 20, output_tokens: output, cache_read_input_tokens: 100, cache_creation_input_tokens: 10 } } });
  expect(agentContextUsage(value)).toMatchObject({ used: 130, latest: null, cumulative: null });
  result(value, 3, 'turn', { usage: { input_tokens: 40, output_tokens: 8, cache_read_input_tokens: 200, cache_creation_input_tokens: 20 }, model_usage: { main: { ...model, outputTokens: 8 } }, model_limits: { main: { contextWindow: 1000 } } });
  expect(agentContextUsage(value)).toMatchObject({ used: 130, limit: 1000, latest: { input: 260, output: 8, total: 268 }, cumulative: { input: 130, output: 8, total: 138 } });
  value.events.push({ seq: 4, kind: 'message', run: 'turn', text: null, data: { type: 'assistant', parent_tool_use_id: 'spawn', usage_source: 'native_response_input', usage: { input_tokens: 99999, output_tokens: 99999 } } });
  expect(agentContextUsage(value).used).toBe(130);
  result(value, 5, 'turn', { usage: { input_tokens: 40, output_tokens: 8, cache_read_input_tokens: 200, cache_creation_input_tokens: 20 }, model_usage: { main: { ...model, outputTokens: 8 } } });
  expect(agentContextUsage(value).latest?.total).toBe(268);
});

it('never substitutes a final turn or cumulative output total for missing context input', () => {
  const value = session();
  value.events.push({ seq: 1, kind: 'message', run: 'turn', text: null, data: { type: 'assistant', usage_source: 'native_response_input', usage: { output_tokens: 99999 } } });
  result(value, 2, 'turn', { usage: { input_tokens: 8000, output_tokens: 9000 }, model_usage: { main: model } });
  expect(agentContextUsage(value)).toMatchObject({ used: null, kind: 'unavailable', latest: { input: 8000, output: 9000 }, cumulative: { total: 135 } });
});

it('retains cumulative native totals after truncation but cannot invent missing duration', () => {
  const value = session(); value.history_truncated = true;
  value.events.push({ seq: 90, kind: 'message', text: null, data: { type: 'assistant', history: true, usage_source: 'native_response', usage: { input_tokens: 999, output_tokens: 99 } } });
  result(value, 91, 'latest', { model_usage: { main: model }, total_cost_usd: 0.2, duration_ms: 40 });
  expect(agentContextUsage(value)).toMatchObject({ used: null, cumulative: { total: 135 }, costUsd: 0.2, durationMs: null });
});

it('rejects guessed model rates and missing or invalid native fields', () => {
  const value = session();
  result(value, 1, 'first', { model_usage: { main: { ...model, costBasis: 'unknown' } }, total_cost_usd: 1 });
  expect(agentContextUsage(value)).toMatchObject({ costUsd: null, durationMs: null });
  result(value, 2, 'second', { model_usage: { main: { ...model, inputTokens: Number.MAX_SAFE_INTEGER } }, total_cost_usd: 1 });
  expect(agentContextUsage(value).cumulative?.input).toBeNull();
  result(value, 3, 'third', { total_cost_usd: 99 });
  expect(agentContextUsage(value)).toMatchObject({ cumulative: null, costUsd: null });
});

it('renders unknown as dashes without a zero progress receipt', () => {
  render(<AgentContextDetails usage={agentContextUsage()} />);
  expect(screen.getByRole('progressbar').getAttribute('aria-valuenow')).toBeNull();
  expect(screen.getByText('agentUsage.cost').nextElementSibling?.textContent).toBe('—');
  expect(screen.getByRole('button', { name: 'agentUsage.compress' })).toHaveProperty('disabled', true);
});

function own(value: AgentSession) {
  useDevelopment.setState({ workspaceOwner: 'account:alice', projects: [{ id: 'project', owner: 'alice', name: 'Project', path: '/workspace', directory_identity: 'directory', archived: false, created_at: 1, last_opened_at: null }], sessions: [{ id: value.id, owner: 'alice', project_id: 'project', task_id: 'task', engine: value.engine, native_id: value.native_id, workspace: value.workspace, executable: null, model: null }] });
  useAgentRunner.setState({ session: value, selected: value.id, snapshots: { [value.id]: value }, live: {} });
}

it('dispatches real Claude compact, retains receipts, and checks uncertain requests without replay', async () => {
  const value = session(); own(value);
  value.events.push({ seq: 1, kind: 'message', run: 'previous', text: null, data: { type: 'assistant', usage_source: 'native_response', usage: { input_tokens: 20, output_tokens: 5 } } });
  let request: { request_id: string; session_id: string; generation: string } | undefined;
  invoke.mockImplementation(async (command: string, args: { input?: typeof request }) => {
    if (command === 'observe_agent_compression') return true;
    if (command === 'control_codex_operation') { request = args.input; throw 'transport_lost'; }
    if (command === 'inspect_codex_operation') return { ...request, state: 'applied' };
    if (command === 'get_agent_session') return value;
    throw 'unexpected_command';
  });
  render(<AgentContextDetails session={value} usage={agentContextUsage(value)} />);
  const button = screen.getByRole('button', { name: 'agentUsage.compress' });
  await waitFor(() => expect(button).toHaveProperty('disabled', false));
  fireEvent.click(button);
  await screen.findByText('agentUsage.unknown');
  expect(screen.getByText('20 / —')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'agentUsage.check' }));
  await waitFor(() => expect(screen.queryByText('agentUsage.unknown')).toBeNull());
  expect(invoke.mock.calls.filter(([command]) => command === 'control_codex_operation')).toHaveLength(1);
  expect(invoke.mock.calls.find(([command]) => command === 'control_codex_operation')?.[1]).toMatchObject({ input: { session_id: value.id, generation: value.generation, action: { kind: 'compact' } } });
});

it('disables compression during a run and discards late capability across account changes', async () => {
  const value = session(); own(value);
  const capability = Promise.withResolvers<boolean>(); invoke.mockReturnValue(capability.promise);
  const view = render(<AgentContextDetails session={value} usage={agentContextUsage(value)} />);
  act(() => useDevelopment.setState(state => ({ epoch: state.epoch + 1, workspaceOwner: 'account:bob', projects: [], sessions: [] })));
  await act(async () => capability.resolve(true));
  expect(useCodexOperations.getState().entries).toEqual({});
  view.unmount();
  own({ ...value, status: 'running', active_run: 'turn' }); invoke.mockResolvedValue(true);
  render(<AgentContextDetails session={{ ...value, status: 'running', active_run: 'turn' }} usage={agentContextUsage(value)} />);
  await waitFor(() => expect(screen.getByRole('button', { name: 'agentUsage.compress' })).toHaveProperty('disabled', true));
  fireEvent.click(screen.getByRole('button', { name: 'agentUsage.compress' }));
  expect(invoke.mock.calls.some(([command]) => command === 'control_codex_operation')).toBe(false);
});

it('observes compression after an overlapping operation read settles instead of staying disabled', async () => {
  const value = session(); own(value);
  const key = codexOperationsKey({ epoch: useDevelopment.getState().epoch, accountKey: 'account:alice', owner: 'alice', projectId: 'project', taskId: 'task', sessionId: value.id, generation: value.generation });
  useCodexOperations.setState({ entries: { [key]: { ...emptyCodexOperations, reading: true } } });
  invoke.mockResolvedValue(true);
  render(<AgentContextDetails session={value} usage={agentContextUsage(value)} />);
  expect(invoke).not.toHaveBeenCalled();
  act(() => useCodexOperations.setState(state => ({ entries: { ...state.entries, [key]: { ...state.entries[key], reading: false } } })));
  await waitFor(() => expect(screen.getByRole('button', { name: 'agentUsage.compress' })).toHaveProperty('disabled', false));
  expect(invoke.mock.calls.filter(([command]) => command === 'observe_agent_compression')).toHaveLength(1);
});

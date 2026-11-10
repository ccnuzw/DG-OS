import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { AgentComposerControls } from '../src/components/development/AgentComposerControls';
import { agentContextUsage } from '../src/components/development/agentContextUsage';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { agentSettingsKey, agentTurnSettings } from '../src/stores/agentTurnSettings';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
import { CapabilityReturn } from '../src/components/chat/RuntimeNavigation';
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); });
const session = (id = 'one'): AgentSession => ({ id, generation: `generation-${id}`, engine: 'codex', native_id: id, workspace: '/fixture', status: 'idle', pending: [], events: [], error: null, controls: { reasoning: { available: true, options: ['low', 'high'], current: 'low', source: 'model/list', reason: null }, permission: { available: true, options: ['read_only', 'default'], current: 'read_only', source: 'turn/start', reason: null } } });
function select(value: AgentSession) { useAgentRunner.setState({ session: value, selected: value.id }); useDevelopment.setState({ selection: { project_id: 'project', task_id: 'task', session_id: value.id } }); }
const controls = (value: AgentSession) => <MemoryRouter><AgentComposerControls projectId="project" session={value} model={<span>Bound model</span>} /></MemoryRouter>;
it('requires explicit privilege expansion and isolates next-turn intent from other sessions and generations', () => {
  const first = session(); select(first); const view = render(controls(first));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission.default' }));
  expect(agentTurnSettings(first)).toBeUndefined();
  fireEvent.click(screen.getByRole('button', { name: 'common.cancel' }));
  expect(agentTurnSettings(first)).toBeUndefined();
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission.default' }));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.confirmExpansion' }));
  expect(agentTurnSettings(first)).toEqual({ permission_mode: 'default' });
  expect(first.controls?.permission.current).toBe('read_only');
  const second = session('two'); act(() => select(second)); view.rerender(controls(second));
  expect(agentTurnSettings(second)).toBeUndefined();
  act(() => select(first)); view.rerender(controls(first));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.reasoning' }));
  fireEvent.click(screen.getByRole('button', { name: 'high', exact: true }));
  expect(agentTurnSettings(first)).toEqual({ reasoning_effort: 'high', permission_mode: 'default' });
  expect(agentTurnSettings({ ...first, generation: 'replacement' })).toBeUndefined();
  act(() => useDevelopment.getState().reset());
  expect(agentTurnSettings(first)).toBeUndefined();
});
it('cannot modify running or pending settings and never offers absent model effort values', () => {
  const value = { ...session(), status: 'waiting', pending: [{ id: 'pending', kind: 'approval', title: 'original request', data: {} }] };
  select(value); render(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.reasoning' }));
  const choices = within(screen.getByRole('group', { name: 'agentControls.reasoning' }));
  expect(choices.queryByRole('button', { name: 'xhigh' })).toBeNull();
  expect(choices.getByRole('button', { name: 'high' })).toHaveProperty('disabled', true);
  fireEvent.click(choices.getByRole('button', { name: 'high' }));
  expect(agentTurnSettings(value)).toBeUndefined();
  expect(value.pending[0].id).toBe('pending');
});
it('uses latest Codex turn usage, never cumulative totals, and refuses guessed context limits', () => {
  const value = session();
  value.events = [{ seq: 1, kind: 'thread/tokenUsage/updated', text: null, data: { tokenUsage: { last: { inputTokens: 200, outputTokens: 30 }, total: { totalTokens: 90000 }, modelContextWindow: 1000 } } }];
  expect(agentContextUsage(value)).toMatchObject({ used: 230, limit: 1000, kind: 'estimated' });
  value.events.push({ seq: 2, kind: 'thread/tokenUsage/updated', text: null, data: { tokenUsage: { total: { totalTokens: 99000 } } } });
  expect(agentContextUsage(value)).toMatchObject({ used: null, limit: null, kind: 'unavailable' });
});
it('keeps Claude context to the latest main-agent response and exact same-run model limit', () => {
  const value = { ...session(), engine: 'claude' as const };
  value.events = [
    { seq: 1, kind: 'message', text: null, run: 'turn', data: { type: 'assistant', model: 'native-model', usage_source: 'native_response', usage: { input_tokens: 20, output_tokens: 5, cache_read_input_tokens: 100, cache_creation_input_tokens: 10 } } },
    { seq: 2, kind: 'message', text: null, run: 'turn', data: { type: 'assistant', model: 'native-model', parent_tool_use_id: 'subagent', usage_source: 'native_response', usage: { input_tokens: 9999, output_tokens: 100 } } },
    { seq: 3, kind: 'result', text: null, run: 'turn', data: { usage: { input_tokens: 80000 }, model_limits: { 'native-model': { contextWindow: 1000 } } } },
  ];
  expect(agentContextUsage(value)).toMatchObject({ used: 135, limit: 1000, seq: 1 });
  value.events[2].run = 'another-turn';
  expect(agentContextUsage(value).limit).toBeNull();
  value.events[0].data = { ...(value.events[0].data as object), history: true };
  expect(agentContextUsage(value).used).toBeNull();
});

it('does not invent a Claude context limit without a run identity or label overflow as an estimate', () => {
  const value: AgentSession = { ...session(), engine: 'claude', events: [
    { seq: 1, kind: 'message', text: null, data: { type: 'assistant', model: 'm', usage_source: 'native_response', usage: { input_tokens: 4, output_tokens: 2 } } },
    { seq: 2, kind: 'result', text: null, data: { model_limits: { m: { contextWindow: 1000 } } } },
  ] };
  expect(agentContextUsage(value)).toMatchObject({ used: 6, limit: null, kind: 'estimated' });
  value.events[0].data = { type: 'assistant', model: 'm', usage_source: 'native_response', usage: { input_tokens: Number.MAX_SAFE_INTEGER, output_tokens: 1 } };
  expect(agentContextUsage(value)).toMatchObject({ used: null, limit: null, kind: 'unavailable' });
});

it('returns to the original tab without creating a runtime and refuses a closed origin', async () => {
  const first = useDevelopment.getState().newTab('project-one');
  const second = useDevelopment.getState().newTab('project-two');
  useDevelopment.setState({ selection: { project_id: second.projectId, task_id: null, session_id: null } });
  function ReturnSurface() { const location = useLocation(); return <><output data-testid="path">{location.pathname}</output><CapabilityReturn path="/skills" /></>; }
  const state = { capabilityEntry: { runtime: 'native', selection: { target: 'Codex', scope: 'ProjectShared', project_id: first.projectId }, returnTabId: first.id, epoch: useDevelopment.getState().epoch } };
  const view = render(<MemoryRouter initialEntries={[{ pathname: '/skills', state }]}><ReturnSurface /></MemoryRouter>);
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.returnConversation' }));
  await waitFor(() => expect(screen.getByTestId('path').textContent).toBe('/tasks/agent'));
  expect(useDevelopment.getState().activeTabId).toBe(first.id);
  expect(useDevelopment.getState().selection.project_id).toBe('project-one');
  expect(useAgentRunner.getState().live).toEqual({});
  view.unmount();
  useDevelopment.getState().closeTab(first.id);
  render(<MemoryRouter initialEntries={[{ pathname: '/skills', state }]}><ReturnSurface /></MemoryRouter>);
  expect(screen.getByRole('button', { name: 'agentControls.returnConversation' })).toHaveProperty('disabled', true);
  expect(useDevelopment.getState().activeTabId).toBe(second.id);
});

it('expires privilege confirmation when native capability or waiting state changes', () => {
  const value = session(); select(value); const view = render(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission.default' }));
  expect(screen.getByRole('button', { name: 'agentControls.confirmExpansion' })).toBeTruthy();
  const waiting: AgentSession = { ...value, status: 'waiting', pending: [{ id: 'approval', kind: 'approval', title: 'New scope', data: {} }] };
  act(() => select(waiting)); view.rerender(controls(waiting));
  expect(screen.queryByRole('button', { name: 'agentControls.confirmExpansion' })).toBeNull();
  act(() => select(value)); view.rerender(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  expect(screen.queryByRole('button', { name: 'agentControls.confirmExpansion' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission.default' }));
  const confirm = screen.getByRole('button', { name: 'agentControls.confirmExpansion' });
  // A retained callback must consult live authority, not just the rendered controls.
  act(() => useAgentRunner.setState({ session: { ...value, controls: { ...value.controls!, permission: { ...value.controls!.permission, available: false, options: [] } } } }));
  fireEvent.click(confirm);
  expect(agentTurnSettings(value)).toBeUndefined();
});

it('merges a reasoning edit into latest sparse draft intent instead of overwriting concurrent fields', () => {
  const value = session(); select(value); render(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.reasoning' }));
  const high = screen.getByRole('button', { name: 'high', exact: true });
  act(() => {
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSettingsKey(value)]: { kind: 'conversation', value: { permission_mode: 'read_only' }, baseline: {} } } }));
    fireEvent.click(high);
  });
  expect(agentTurnSettings(value)).toEqual({ reasoning_effort: 'high', permission_mode: 'read_only' });
});

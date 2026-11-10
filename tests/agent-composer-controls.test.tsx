import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { MemoryRouter } from 'react-router-dom';
import { AgentComposerControls } from '../src/components/development/AgentComposerControls';
import { agentContextUsage } from '../src/components/development/agentContextUsage';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { agentSettingsKey, agentTurnSettings, resolveAgentDefaults, setAgentDefaults } from '../src/stores/agentTurnSettings';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { setDesktopSlot } from '../src/stores/developmentDesktop';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); });
const session = (id = 'one'): AgentSession => ({ id, generation: `generation-${id}`, engine: 'codex', native_id: id, workspace: '/fixture', status: 'idle', pending: [], events: [], error: null, controls: { reasoning: { available: true, options: ['low', 'high'], current: 'low', source: 'model/list', reason: null }, permission: { available: true, options: ['read_only', 'default'], current: 'read_only', source: 'turn/start', reason: null } } });
function select(value: AgentSession) { useAgentRunner.setState({ session: value, selected: value.id }); useDevelopment.setState({ selection: { project_id: 'project', task_id: 'task', session_id: value.id } }); }
const controls = (value: AgentSession) => <MemoryRouter><AgentComposerControls projectId="project" session={value} model={<span>Bound model</span>} /></MemoryRouter>;
it('direct mode selection retains session intent and isolates other sessions', () => {
  const first = session(); select(first); const view = render(controls(first));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  fireEvent.click(screen.getByRole('button', { name: /agentControls.mode.default/ }));
  expect(agentTurnSettings(first)).toEqual({ permission_mode: 'default' });
  expect(first.controls?.permission.current).toBe('read_only');
  const second = session('two'); act(() => select(second)); view.rerender(controls(second));
  expect(agentTurnSettings(second)).toBeUndefined();
  act(() => select(first)); view.rerender(controls(first));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.reasoning' }));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.effort.high', exact: true }));
  expect(agentTurnSettings(first)).toEqual({ reasoning_effort: 'high', permission_mode: 'default' });
  expect(agentTurnSettings({ ...first, generation: 'replacement' })).toEqual({ reasoning_effort: 'high', permission_mode: 'default' });
  act(() => useDevelopment.getState().reset());
  expect(agentTurnSettings(first)).toBeUndefined();
});
it('cannot modify running or pending settings and never offers absent model effort values', () => {
  const value = { ...session(), status: 'waiting', pending: [{ id: 'pending', kind: 'approval', title: 'original request', data: {} }] };
  select(value); render(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.reasoning' }));
  const choices = within(screen.getByRole('group', { name: 'agentControls.reasoning' }));
  expect(choices.queryByRole('button', { name: 'agentControls.effort.xhigh' })).toBeNull();
  expect(choices.getByRole('button', { name: 'agentControls.effort.high' })).toHaveProperty('disabled', true);
  fireEvent.click(choices.getByRole('button', { name: 'agentControls.effort.high' }));
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


it('rejects a retained mode callback when live capabilities change', () => {
  const value = session(); select(value); render(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  const choose = screen.getByRole('button', { name: /agentControls.mode.default/ });
  act(() => useAgentRunner.setState({ session: { ...value, controls: { ...value.controls!, permission: { ...value.controls!.permission, available: false, options: [] } } } }));
  fireEvent.click(choose);
  expect(agentTurnSettings(value)).toBeUndefined();
});

it('merges a reasoning edit into latest sparse draft intent instead of overwriting concurrent fields', () => {
  const value = session(); select(value); render(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.reasoning' }));
  const high = screen.getByRole('button', { name: 'agentControls.effort.high', exact: true });
  act(() => {
    useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [agentSettingsKey(value)]: { kind: 'conversation', value: { permission_mode: 'read_only' }, baseline: {} } } }));
    fireEvent.click(high);
  });
  expect(agentTurnSettings(value)).toEqual({ reasoning_effort: 'high', permission_mode: 'read_only' });
});

it('selects planning independently from reasoning and restores execution through a direct mode choice', () => {
  const value = session();
  value.controls!.collaboration = { available: true, options: ['default', 'plan', 'deep'], current: 'default', source: 'turn/start', reason: null };
  select(value); render(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  fireEvent.click(screen.getByRole('button', { name: /agentControls.mode.deep/ }));
  expect(agentTurnSettings(value)).toEqual({ collaboration_mode: 'deep' });
  expect(value.controls!.reasoning.current).toBe('low');
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  fireEvent.click(screen.getByRole('button', { name: /agentControls.mode.default/ }));
  expect(agentTurnSettings(value)).toEqual({ collaboration_mode: 'default', permission_mode: 'default' });
});

it('shows restored full automation intent rather than the native default and allows a safe override', () => {
  const value = session();
  value.controls!.permission = { ...value.controls!.permission, options: ['default', 'full_auto'], current: 'default' };
  select(value);
  setDesktopSlot(`agent-session:${value.id}`, { configuration: null, settings: { permission_mode: 'full_auto', reasoning_effort: 'high' } });
  render(controls(value));
  expect(screen.getByRole('button', { name: 'agentControls.permission' }).textContent).toContain('agentControls.mode.full_auto');
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  expect(screen.getByRole('button', { name: /agentControls.mode.full_auto/ }).getAttribute('aria-pressed')).toBe('true');
  fireEvent.click(screen.getByRole('button', { name: /agentControls.mode.default/ }));
  expect(agentTurnSettings(value)).toMatchObject({ permission_mode: 'default', reasoning_effort: 'high' });
});

it.each(['project', 'application'])('never saves full automation as a %s default', scope => {
  setAgentDefaults(scope, { configuration: null, settings: { permission_mode: 'full_auto', collaboration_mode: 'deep', reasoning_effort: 'high' } });
  expect(resolveAgentDefaults(scope === 'project' ? scope : null).settings).toEqual({ permission_mode: 'default', collaboration_mode: 'deep', reasoning_effort: 'high' });
});

it('authorizes full automation only for the explicitly selected session intent', () => {
  const value = session();
  value.controls!.permission = { ...value.controls!.permission, current: 'default', options: ['default', 'full_auto'] };
  select(value);
  const view = render(controls(value));
  fireEvent.click(screen.getByRole('button', { name: 'agentControls.permission' }));
  fireEvent.click(screen.getByRole('button', { name: /agentControls.mode.full_auto/ }));
  expect(agentTurnSettings(value)).toEqual({ permission_mode: 'full_auto' });
  expect(value.controls!.permission.current).toBe('default');
  expect(screen.queryByRole('group', { name: 'agentControls.runMode' })).toBeNull();
  const other = session('other');
  act(() => select(other)); view.rerender(controls(other));
  expect(agentTurnSettings(other)).toBeUndefined();
});

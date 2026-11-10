import { useState } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { CodexTurnControls } from '../src/components/development/CodexTurnControls';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
import { agentTurnSettings } from '../src/stores/agentTurnSettings';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
let native: AgentSession;
beforeEach(() => {
  native = { id: 'session', generation: 'generation', engine: 'codex', workspace: '/tmp/project', native_id: 'native', status: 'running', active_run: 'run-1', events: [], pending: [], error: null, controls: {
    reasoning: { available: true, options: ['low'], current: 'low', source: 'native', reason: null },
    permission: { available: true, options: ['default'], current: 'default', source: 'native', reason: null },
    collaboration: { available: true, options: ['default', 'plan'], current: 'default', source: 'native', reason: null }, steering: true,
  } };
  useAgentRunner.setState({ session: native, snapshots: { [native.id]: native }, selected: native.id, busy: false, acting: false });
  invoke.mockImplementation(async (command: string, args: { input?: { request_id: string; run_id: string } }) => command === 'get_agent_session' ? native : command === 'steer_agent_turn' ? { request_id: args.input!.request_id, run_id: args.input!.run_id, state: 'applied' } : null);
});
afterEach(() => { cleanup(); useAgentRunner.getState().reset(); useDevelopment.getState().reset(); useDevelopmentDrafts.getState().clear(); invoke.mockReset(); });
function Surface() {
  const session = useAgentRunner(state => state.session)!;
  const [prompt, setPrompt] = useState('additional instruction');
  return <><input aria-label="draft" value={prompt} onChange={event => setPrompt(event.target.value)} /><CodexTurnControls session={session} projectId="project" prompt={prompt} setDraft={(_id, value, expected) => setPrompt(current => expected === undefined || current === expected ? value : current)} currentOwner={() => useAgentRunner.getState().selected === session.id} /></>;
}
it('retains unknown steering through terminal and rechecks without resending', async () => {
  invoke.mockImplementation(async (command: string, args: { input?: { request_id: string; run_id: string } }) => command === 'get_agent_session' ? native : { ...args.input, state: 'unknown' });
  render(<Surface />);
  fireEvent.click(screen.getByText('codexInteractions.steer'));
  await screen.findByText('codexInteractions.unknown');
  native = { ...native, status: 'idle', active_run: null };
  fireEvent.click(screen.getByText('codexInteractions.recheck'));
  await waitFor(() => expect(useAgentRunner.getState().session?.status).toBe('idle'));
  expect((screen.getByLabelText('draft') as HTMLInputElement).value).toBe('additional instruction');
  expect(screen.getByText('codexInteractions.unknown')).toBeTruthy();
  expect(invoke.mock.calls.filter(([name]) => name === 'steer_agent_turn')).toHaveLength(1);
});
it('only acknowledges the captured draft, preserving edits made while steering is in flight', async () => {
  let finish!: (value: unknown) => void;
  invoke.mockImplementation(async (command: string) => command === 'steer_agent_turn' ? await new Promise(resolve => { finish = resolve; }) : native);
  render(<Surface />);
  fireEvent.click(screen.getByText('codexInteractions.steer'));
  const args = invoke.mock.calls.find(([name]) => name === 'steer_agent_turn')![1].input;
  expect(args).toMatchObject({ session_id: native.id, generation: native.generation, run_id: 'run-1', text: 'additional instruction' });
  fireEvent.change(screen.getByLabelText('draft'), { target: { value: 'next instruction' } });
  await act(async () => { finish({ request_id: args.request_id, run_id: args.run_id, state: 'applied' }); });
  expect((screen.getByLabelText('draft') as HTMLInputElement).value).toBe('next instruction');
  expect(screen.queryByText('codexInteractions.unknown')).toBeNull();
});
it('accepts only matching positive evidence when checking an unknown receipt', async () => {
  invoke.mockImplementation(async (command: string, args: { input?: { request_id: string; run_id: string } }) => command === 'get_agent_session' ? native : { ...args.input, state: 'unknown' });
  render(<Surface />);
  fireEvent.click(screen.getByText('codexInteractions.steer'));
  await screen.findByText('codexInteractions.unknown');
  const args = invoke.mock.calls.find(([name]) => name === 'steer_agent_turn')![1].input;
  native = { ...native, events: [{ seq: 1, kind: 'userSteering', text: args.text, run: 'other-run', data: { requestId: args.request_id } }] };
  await act(async () => { await useAgentRunner.getState().attach(native.id); });
  expect((screen.getByLabelText('draft') as HTMLInputElement).value).toBe(args.text);
  native = { ...native, events: [{ seq: 2, kind: 'userSteering', text: args.text, run: args.run_id, data: { requestId: args.request_id } }] };
  await act(async () => { await useAgentRunner.getState().attach(native.id); });
  expect((screen.getByLabelText('draft') as HTMLInputElement).value).toBe('');
  expect(screen.queryByText('codexInteractions.unknown')).toBeNull();
});
it('keeps mode choice as next-turn intent and never changes acknowledged native state optimistically', () => {
  native = { ...native, status: 'idle', active_run: null };
  useAgentRunner.setState({ session: native });
  render(<Surface />);
  fireEvent.click(screen.getByRole('button', { name: 'codexInteractions.plan' }));
  expect(agentTurnSettings(native)).toEqual({ collaboration_mode: 'plan' });
  expect(useAgentRunner.getState().session?.controls?.collaboration?.current).toBe('default');
  expect(screen.getByText('codexInteractions.nextTurn')).toBeTruthy();
  expect(invoke).not.toHaveBeenCalled();
});
it('rejects stale run callbacks before native dispatch and does not allow unsupported steering', async () => {
  const oldInput = { session_id: native.id, generation: native.generation, run_id: 'old-run', request_id: 'request', text: 'old' };
  expect(await useAgentRunner.getState().action('steer_agent_turn', { input: oldInput })).toBe(false);
  native = { ...native, controls: { ...native.controls!, steering: false } };
  useAgentRunner.setState({ session: native });
  render(<Surface />);
  expect((screen.getByRole('button', { name: 'codexInteractions.steer' }) as HTMLButtonElement).disabled).toBe(true);
  expect(invoke).not.toHaveBeenCalled();
});

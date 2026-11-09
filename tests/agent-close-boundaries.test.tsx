import { afterEach, expect, it, vi } from 'vitest';
import { prepareAgentClose } from '../src/workspace/closeCoordinator';
import { useAgentRunner, type AgentOpenInput, type AgentSession } from '../src/stores/agentRunnerStore';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ t: (key: string) => key }));
afterEach(() => { useAgentRunner.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });
const session = (id: string): AgentSession => ({ id, generation: `generation-${id}`, engine: 'codex', native_id: id, workspace: '/tmp/project', status: 'idle', events: [], pending: [], error: null });
const input: AgentOpenInput = { intent: 'create', id: 'new', engine: 'codex', task_id: 'task', project_revision: 1, provider: { provider_id: 'provider', model_id: 'model', provider_revision: 0 }, options: { workspace: '/tmp/project', resume_id: null, executable: null, model: null, trust_workspace: true } };

it('hides a connecting projection immediately and tracks its late same-account open without selecting it', async () => {
  const opened = Promise.withResolvers<AgentSession>();
  invoke.mockImplementation(async (command: string, args?: { sessionId: string }) => command === 'open_agent_session' ? opened.promise : session(args!.sessionId));
  const opening = useAgentRunner.getState().open(input);
  expect(useAgentRunner.getState().busy).toBe(true);
  expect(await prepareAgentClose()).toBe(true);
  expect(useAgentRunner.getState().selected).toBeNull();
  expect(useAgentRunner.getState().busy).toBe(false);
  opened.resolve(session('new'));
  await opening;
  expect(useAgentRunner.getState().session).toBeNull();
  expect(useAgentRunner.getState().selected).toBeNull();
  expect(useAgentRunner.getState().live).toEqual({ new: 'idle' });
  expect(useAgentRunner.getState().snapshots.new).toEqual(session('new'));
  expect(useAgentRunner.getState().unread.new).toBe(true);
  expect(invoke.mock.calls.some(([command]) => command === 'close_agent_session' || command === 'interrupt_agent_session')).toBe(false);
});

it('does not wait for an open acknowledgement or a terminal runtime when hiding the surface', async () => {
  useAgentRunner.setState({ busy: true, session: session('old'), selected: 'old', live: { old: 'running', detached: 'waiting' }, snapshots: { old: session('old') } });
  expect(await prepareAgentClose()).toBe(true);
  expect(invoke).not.toHaveBeenCalled();
  expect(useAgentRunner.getState().session).toBeNull();
  expect(useAgentRunner.getState().live).toEqual({ old: 'running', detached: 'waiting' });
  expect(useAgentRunner.getState().snapshots.old).toEqual(session('old'));
});

it('continues observing every runtime after the surface closes', async () => {
  vi.useFakeTimers();
  let completed = false;
  invoke.mockImplementation(async (_command: string, args: { sessionId: string }) => ({ ...session(args.sessionId), status: completed ? 'idle' : 'running', events: completed ? [{ seq: 1, kind: 'output', text: `${args.sessionId} done`, data: null }] : [] }));
  await useAgentRunner.getState().attach('first');
  await useAgentRunner.getState().attach('second');
  useAgentRunner.getState().markSeen('first');
  useAgentRunner.getState().markSeen('second');
  expect(await prepareAgentClose()).toBe(true);
  completed = true;
  await vi.advanceTimersByTimeAsync(600);
  expect(useAgentRunner.getState().selected).toBeNull();
  expect(useAgentRunner.getState().session).toBeNull();
  expect(useAgentRunner.getState().live).toEqual({ first: 'idle', second: 'idle' });
  expect(useAgentRunner.getState().snapshots.first.events[0]?.text).toBe('first done');
  expect(useAgentRunner.getState().snapshots.second.events[0]?.text).toBe('second done');
  expect(useAgentRunner.getState().unread).toEqual({ first: true, second: true });
  expect(invoke.mock.calls.every(([command]) => command === 'get_agent_session')).toBe(true);
});

it('keeps explicit runtime termination separate from surface close and rejects pre-close reads', async () => {
  vi.useFakeTimers();
  const stale = Promise.withResolvers<AgentSession>();
  let reads = 0;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'get_agent_session') return ++reads === 1 ? session('runtime') : stale.promise;
    if (command === 'close_agent_session') return true;
    throw new Error(command);
  });
  await useAgentRunner.getState().attach('runtime');
  await vi.advanceTimersByTimeAsync(600);
  expect(await useAgentRunner.getState().action('close_agent_session', { sessionId: 'runtime' })).toBe(true);
  stale.resolve({ ...session('runtime'), status: 'running', pending: [{ id: 'stale', kind: 'approval', title: 'old', data: null }] });
  await Promise.resolve(); await Promise.resolve();
  expect(useAgentRunner.getState().live).toEqual({});
  expect(useAgentRunner.getState().session?.status).toBe('closed');
  expect(useAgentRunner.getState().snapshots.runtime.pending).toEqual([]);
  expect(invoke).toHaveBeenCalledWith('close_agent_session', { sessionId: 'runtime' });
  const before = invoke.mock.calls.length;
  await vi.advanceTimersByTimeAsync(1800);
  expect(invoke.mock.calls.length).toBe(before);
});

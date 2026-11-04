import { afterEach, expect, it, vi } from 'vitest';
import { prepareAgentClose } from '../src/workspace/closeCoordinator';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
afterEach(() => { useAgentRunner.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });
const session = (id: string): AgentSession => ({ id, engine: 'codex', native_id: id, workspace: '/tmp/project', status: 'idle', events: [], pending: [], error: null });
it('keeps the tab open until a replacement connection settles and closes the replacement runtime', async () => {
  let finishOpen!: (value: AgentSession) => void;
  let closed: string | undefined;
  invoke.mockImplementation(async (command: string, args?: { sessionId: string }) => {
    if (command === 'open_agent_session') return new Promise<AgentSession>(resolve => { finishOpen = resolve; });
    if (command === 'list_agent_sessions') return [];
    if (command === 'close_agent_session') { closed = args!.sessionId; return true; }
    if (command === 'get_agent_session') return { ...session(args!.sessionId), status: closed === args!.sessionId ? 'closed' : 'idle' };
  });
  useAgentRunner.setState({ session: session('old'), selected: 'old' });
  const opening = useAgentRunner.getState().open({ id: 'new', engine: 'codex', task_id: null, options: { workspace: '/tmp/project', resume_id: null, executable: null, model: null, trust_workspace: true } });
  let settled = false;
  const closing = prepareAgentClose().then(value => { settled = true; return value; });
  await Promise.resolve();
  expect(settled).toBe(false);
  expect(closed).toBeUndefined();
  finishOpen(session('new'));
  await opening;
  expect(await closing).toBe(true);
  expect(closed).toBe('new');
  expect(useAgentRunner.getState().session?.status).toBe('closed');
});
it('preserves a connecting tab when the close deadline expires', async () => {
  vi.useFakeTimers();
  useAgentRunner.setState({ busy: true, session: session('old'), selected: 'old' });
  const closing = prepareAgentClose();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(await closing).toBe(false);
  expect(invoke).not.toHaveBeenCalled();
  expect(useAgentRunner.getState().session?.id).toBe('old');
});

it('closes every detached runtime before allowing the surface to close', async () => {
  const closed = new Set<string>();
  invoke.mockImplementation(async (command: string, args: { sessionId: string }) => {
    if (command === 'close_agent_session') { closed.add(args.sessionId); return true; }
    if (command === 'get_agent_session') return { ...session(args.sessionId), status: closed.has(args.sessionId) ? 'closed' : 'running' };
  });
  useAgentRunner.setState({ session: session('selected'), selected: 'selected', live: { selected: 'idle', detached: 'running' } });
  expect(await prepareAgentClose()).toBe(true);
  expect([...closed].sort()).toEqual(['detached', 'selected']);
  expect(useAgentRunner.getState().live).toEqual({});
});

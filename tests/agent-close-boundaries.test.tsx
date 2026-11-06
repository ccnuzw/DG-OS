import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Modal, ModalHost } from '@dgos/ui';
import AgentWorkbench from '../src/pages/workbench/AgentWorkbench';
import { useDevelopment } from '../src/stores/developmentStore';
import { createWorkspaceController } from '../src/workspace/controller';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { afterEach, expect, it, vi } from 'vitest';
import { prepareAgentClose } from '../src/workspace/closeCoordinator';
import { useAgentRunner, type AgentSession } from '../src/stores/agentRunnerStore';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { act(() => Modal.closeAll()); cleanup(); useAgentRunner.getState().reset(); useDevelopment.getState().reset(); invoke.mockReset(); vi.useRealTimers(); });
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

it('confirms an initial pending Agent connection and keeps its surface until the host close settles', async () => {
  const opened = Promise.withResolvers<AgentSession>();
  const terminal = Promise.withResolvers<AgentSession>();
  let closingHost = false;
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_workspace') return { projects: [], tasks: [], sessions: [], selection: { project_id: null, task_id: null, session_id: null }, operations: [] };
    if (command === 'open_agent_session') return opened.promise;
    if (command === 'close_agent_session') { closingHost = true; return true; }
    if (command === 'get_agent_session') return closingHost ? terminal.promise : session('first');
    if (command === 'list_agent_sessions') return [];
    if (command === 'get_development_session_capabilities') return null;
    if (command === 'list_development_capability_confirmations') return [];
  });
  const controller = createWorkspaceController();
  controller.open({ kind: 'task', id: 'cli-agent', title: 'Agent' });
  render(<><SurfaceScope id="cli-agent" active controller={controller}><AgentWorkbench /></SurfaceScope><ModalHost /></>);
  await waitFor(() => expect(useDevelopment.getState().loaded).toBe(true));
  let opening!: Promise<void>;
  act(() => {
    opening = useAgentRunner.getState().open({ id: 'first', engine: 'codex', task_id: null, options: { workspace: '/tmp/project', resume_id: null, executable: null, model: null, trust_workspace: true } });
  });
  expect(useAgentRunner.getState().session).toBeNull();
  expect(useAgentRunner.getState().live).toEqual({});
  expect(controller.getSurfaceStatuses()['cli-agent']?.running).toBe(true);

  let closing!: Promise<boolean>;
  act(() => { closing = controller.requestClose('cli-agent'); });
  const dialog = await screen.findByRole('dialog', { name: 'agent.title' });
  expect(within(dialog).getByText('shell.closeRunningConfirm')).toBeTruthy();
  fireEvent.click(within(dialog).getByRole('button', { name: 'shell.stopAndClose' }));
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(controller.getSnapshot().surfaces.some(surface => surface.id === 'cli-agent')).toBe(true);
  expect(invoke.mock.calls.some(([command]) => command === 'close_agent_session')).toBe(false);

  await act(async () => { opened.resolve(session('first')); await opening; });
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('close_agent_session', { sessionId: 'first' }));
  expect(controller.getSnapshot().surfaces.some(surface => surface.id === 'cli-agent')).toBe(true);
  await act(async () => { terminal.resolve({ ...session('first'), status: 'closed' }); expect(await closing).toBe(true); });

  expect(controller.getSnapshot().surfaces).toEqual([]);
  expect(useAgentRunner.getState().session?.status).toBe('closed');
  expect(invoke.mock.calls.some(([command]) => command === 'stop_development_process')).toBe(false);
});

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { useTaskRunner } from '../src/stores/taskRunnerStore';
import { useAgentRunner, type AgentOpenInput, type AgentSession } from '../src/stores/agentRunnerStore';
import { prepareTaskClose, prepareAgentClose } from '../src/workspace/closeCoordinator';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);
const input = { provider_id: 'p', model_id: 'm', prompt: 'run', skill_ids: [], generation: {}, allow_without_skill: true };
const task = (id: string, status: string) => ({ task: { id, status, provider_id: 'p', model_id: 'm', created_at: 1 }, skills: [] });
const agentInput: AgentOpenInput = { intent: 'create', id: 'agent', engine: 'codex', task_id: 'task', project_revision: 1, provider: { provider_id: 'p', model_id: 'm', provider_revision: 0 }, options: { workspace: '/tmp', resume_id: null, executable: null, model: null, trust_workspace: true } };
beforeEach(() => { vi.useFakeTimers(); useTaskRunner.getState().reset(); useAgentRunner.getState().reset(); });
afterEach(() => { useTaskRunner.getState().reset(); useAgentRunner.getState().reset(); mocked.mockReset(); vi.useRealTimers(); });
it('keeps task ownership after stop ack and closes only when the late terminal arrives', async () => {
  let complete!: (value: unknown) => void;
  mocked.mockImplementation(async command => {
    if (command === 'run_workbench_task') return new Promise(resolve => { complete = resolve; });
    if (command === 'cancel_workbench_task') return true;
    if (command === 'list_workbench_tasks') return [];
    throw Error(command);
  });
  const run = useTaskRunner.getState().run(input, []);
  const id = useTaskRunner.getState().taskId!;
  let closed = false;
  const close = prepareTaskClose().then(value => { closed = value; return value; });
  await vi.advanceTimersByTimeAsync(500);
  expect(closed).toBe(false);
  expect(useTaskRunner.getState().taskId).toBe(id);
  complete({ record: task(id, 'cancelled') });
  await run;
  expect(await close).toBe(true);
});
it('keeps an unconfirmed task attached after close timeout and rejects old-owner completion', async () => {
  let complete!: (value: unknown) => void;
  mocked.mockImplementation(async command => {
    if (command === 'run_workbench_task') return new Promise(resolve => { complete = resolve; });
    if (command === 'cancel_workbench_task') return false;
    if (command === 'list_workbench_tasks') return [];
    throw Error(command);
  });
  const run = useTaskRunner.getState().run(input, []);
  const id = useTaskRunner.getState().taskId!;
  const close = prepareTaskClose();
  await vi.advanceTimersByTimeAsync(10_500);
  expect(await close).toBe(false);
  expect(useTaskRunner.getState().taskId).toBe(id);
  useTaskRunner.getState().reset();
  complete({ record: task(id, 'succeeded'), content: 'old owner output' });
  await run;
  expect(useTaskRunner.getState().outcome).toBeNull();
});
it('hides a starting Agent immediately while retaining its late runtime subscription', async () => {
  const opened = Promise.withResolvers<AgentSession>();
  const session: AgentSession = { id: 'agent', generation: 'generation-agent', engine: 'codex', native_id: 'n', workspace: '/tmp', status: 'running', events: [], pending: [], error: null };
  mocked.mockImplementation(async command => {
    if (command === 'open_agent_session') return opened.promise;
    if (command === 'get_agent_session') return session;
    throw Error(command);
  });
  const opening = useAgentRunner.getState().open(agentInput);
  expect(await prepareAgentClose()).toBe(true);
  expect(useAgentRunner.getState().selected).toBeNull();
  opened.resolve(session);
  await opening;
  expect(useAgentRunner.getState().session).toBeNull();
  expect(useAgentRunner.getState().live.agent).toBe('running');
  expect(useAgentRunner.getState().snapshots.agent).toEqual(session);
  expect(mocked.mock.calls.some(([name]) => name === 'close_agent_session' || name === 'interrupt_agent_session')).toBe(false);
});
it('ignores an old account Agent open response after reset', async () => {
  let finish!: (value: unknown) => void;
  mocked.mockImplementation(async command => {
    if (command === 'open_agent_session') return new Promise(resolve => { finish = resolve; });
    throw Error(command);
  });
  const opening = useAgentRunner.getState().open({ ...agentInput, id: 'old-agent' });
  useAgentRunner.getState().reset();
  finish({ id: 'old-agent', generation: 'generation-old-agent', engine: 'codex', native_id: 'n', workspace: '/tmp', status: 'running', events: [{seq: 1, kind: 'output', text: 'old account output', data: null}], pending: [], error: null });
  await opening;
  expect(useAgentRunner.getState().session).toBeNull();
  expect(useAgentRunner.getState().selected).toBeNull();
  expect(useAgentRunner.getState().snapshots).toEqual({});
  expect(useAgentRunner.getState().unread).toEqual({});
  expect(useAgentRunner.getState().live).toEqual({});
});

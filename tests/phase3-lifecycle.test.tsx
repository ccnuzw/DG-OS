import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { useTaskRunner } from '../src/stores/taskRunnerStore';
import { useAgentRunner } from '../src/stores/agentRunnerStore';
import { prepareTaskClose, prepareAgentClose } from '../src/workspace/closeCoordinator';
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);
const input = { provider_id: 'p', model_id: 'm', prompt: 'run', skill_ids: [], generation: {}, allow_without_skill: true };
const task = (id: string, status: string) => ({ task: { id, status, provider_id: 'p', model_id: 'm', created_at: 1 }, skills: [] });
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
it('waits for an Agent starting ack and confirms the process closure for that session', async () => {
  let opened!: (value: unknown) => void;
  const session = { id: 'agent', engine: 'codex', native_id: 'n', workspace: '/tmp', status: 'running', events: [], pending: [], error: null };
  let status = 'running';
  mocked.mockImplementation(async command => {
    if (command === 'open_agent_session') return new Promise(resolve => { opened = resolve; });
    if (command === 'list_agent_sessions') return [];
    if (command === 'get_agent_session') return { ...session, status };
    if (command === 'close_agent_session') { status = 'closed'; return true; }
    throw Error(command);
  });
  const opening = useAgentRunner.getState().open({ id: 'agent', engine: 'codex', task_id: null, options: { workspace: '/tmp', resume_id: null, executable: null, model: null, trust_workspace: true } });
  const closing = prepareAgentClose();
  expect(mocked.mock.calls.some(([name]) => name === 'close_agent_session')).toBe(false);
  opened(session);
  await opening;
  expect(await closing).toBe(true);
  expect(useAgentRunner.getState().session?.status).toBe('closed');
});
it('ignores an old account Agent open response after reset', async () => {
  let finish!: (value: unknown) => void;
  mocked.mockImplementation(async command => {
    if (command === 'open_agent_session') return new Promise(resolve => { finish = resolve; });
    throw Error(command);
  });
  const opening = useAgentRunner.getState().open({ id: 'old-agent', engine: 'codex', task_id: null, options: { workspace: '/tmp', resume_id: null, executable: null, model: null, trust_workspace: true } });
  useAgentRunner.getState().reset();
  finish({ id: 'old-agent', engine: 'codex', native_id: 'n', workspace: '/tmp', status: 'running', events: [{seq: 1, kind: 'output', text: 'old account output', data: null}], pending: [], error: null });
  await opening;
  expect(useAgentRunner.getState().session).toBeNull();
  expect(useAgentRunner.getState().selected).toBeNull();
});

import { Profiler } from 'react';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { clearToasts, Modal } from '@dgos/ui';
import Workbench from '../src/pages/workbench/SingleTaskWorkbench';
import { setLocaleForTesting } from '../src/i18n/runtime';
import { useTaskRunner, type ToolTaskUpdate } from '../src/stores/taskRunnerStore';
import { SurfaceScope } from '../src/workspace/SurfaceContext';
import { createWorkspaceController } from '../src/workspace/controller';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }));
const mocked = vi.mocked(invoke);
const input = { provider_id: 'p1', model_id: 'chat1', prompt: 'Review the tool call', skill_ids: [], generation: {}, allow_without_skill: true };
const tool = { server_id: 'server-1', tool_name: 'echo', schema_hash: 'b'.repeat(64) };
interface DeferredUpdate { promise: Promise<ToolTaskUpdate>; resolve(value: ToolTaskUpdate): void }
let model: DeferredUpdate;
let cancellation: DeferredUpdate;
let taskId: string;
const record = (status = 'running') => ({ task: { id: taskId, status, provider_id: 'p1', model_id: 'chat1', created_at: 1 }, skills: [], tools: [tool] });
const proposal = (seconds = 20, id = 'proposal-1'): ToolTaskUpdate => ({ record: record(), phase: 'awaiting_confirmation', proposal: {
  ...tool, id, server_name: 'Local tools', arguments: { text: 'Exact host arguments' }, expires_at: Date.now() / 1000 + seconds,
} });
async function flush() { await act(async () => {}); }
async function advance(milliseconds: number) { await act(async () => { await vi.advanceTimersByTimeAsync(milliseconds); }); }
function show(active = true, covered = false) {
  const controller = createWorkspaceController();
  const commits = vi.fn();
  const tree = (isActive: boolean, isCovered: boolean) => <MemoryRouter future={{ v7_startTransition: true, v7_relativeSplatPath: true }}>
    <button type="button">Keep my focus</button>
    {isCovered && <div role="dialog" aria-label="Settings overlay"><button type="button">Overlay control</button></div>}
    <SurfaceScope id="single-task" active={isActive} controller={controller}>
      <div hidden={!isActive && !isCovered}><Profiler id="single-task" onRender={commits}><Workbench /></Profiler></div>
    </SurfaceScope>
  </MemoryRouter>;
  const view = render(tree(active, covered));
  return { controller, commits, setActive: async (value: boolean, overlay = false) => { view.rerender(tree(value, overlay)); await flush(); } };
}
async function start() {
  let pending!: Promise<void>;
  await act(async () => { pending = useTaskRunner.getState().run(input, [tool]); });
  return { pending };
}
async function arrive(update = proposal()) {
  await act(async () => { model.resolve(update); });
}
const approve = () => screen.getByRole('button', { name: 'Approve this call once', hidden: true }) as HTMLButtonElement;
const countdown = () => screen.getByText(/seconds remaining/);

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-11-05T12:00:00Z'));
  setLocaleForTesting('en-US');
  useTaskRunner.getState().reset();
  model = Promise.withResolvers<ToolTaskUpdate>();
  cancellation = Promise.withResolvers<ToolTaskUpdate>();
  mocked.mockImplementation(async (command, args) => {
    if (command === 'list_providers') return { success: true, data: [] };
    if (command === 'list_installed_skills' || command === 'list_mcp_servers' || command === 'list_workbench_tasks') return [];
    if (command === 'start_workbench_tool_task') {
      const taskInput = args?.input;
      if (!taskInput || typeof taskInput !== 'object' || !('task' in taskInput)) throw new Error('Missing task input');
      const task = taskInput.task;
      if (!task || typeof task !== 'object' || !('client_task_id' in task) || typeof task.client_task_id !== 'string') throw new Error('Missing client task ID');
      taskId = task.client_task_id;
      return { record: record(), phase: 'ready' };
    }
    if (command === 'advance_workbench_tool_task') {
      const action = args?.action;
      if (action && typeof action === 'object' && 'type' in action && action.type === 'run_model') return model.promise;
      return { record: record('succeeded'), phase: 'finished', content: 'Approved host result' };
    }
    if (command === 'cancel_workbench_tool_task') return cancellation.promise;
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(() => {
  cleanup();
  useTaskRunner.getState().reset();
  clearToasts();
  vi.restoreAllMocks();
  mocked.mockReset();
  vi.useRealTimers();
});

it('focuses an approval arriving on the active surface once and preserves exact host approval dispatch', async () => {
  show(); await flush();
  await start();
  const focus = vi.spyOn(HTMLElement.prototype, 'focus');
  await arrive();
  const proposalHeading = screen.getByRole('heading', { name: 'Review proposed tool call' });
  expect(document.activeElement).toBe(proposalHeading);
  expect(focus.mock.contexts.filter(target => target === proposalHeading)).toHaveLength(1);
  screen.getByRole('button', { name: 'Keep my focus' }).focus();
  await advance(3000);
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Keep my focus' }));
  expect(focus.mock.contexts.filter(target => target === proposalHeading)).toHaveLength(1);
  fireEvent.click(approve());
  await flush();
  expect(mocked).toHaveBeenCalledWith('advance_workbench_tool_task', { taskId, action: { type: 'approve', proposal_id: 'proposal-1' } });
  expect(screen.getByText('Approved host result')).toBeTruthy();
});

it('tracks replacement proposal deadlines while focusing only each new active approval', async () => {
  const view = show(); await flush(); await start(); await arrive(proposal(2));
  await advance(2000);
  expect(approve().disabled).toBe(true);
  const control = screen.getByRole('button', { name: 'Keep my focus' });
  control.focus();
  // Host updates are projected through the same subscribed store without restarting the task.
  await act(async () => { useTaskRunner.setState({ toolTask: proposal(6, 'proposal-2') }); });
  const proposalHeading = screen.getByRole('heading', { name: 'Review proposed tool call' });
  expect(document.activeElement).toBe(proposalHeading);
  expect(approve().disabled).toBe(false);
  expect(countdown().textContent).toMatch(/^6 /);
  control.focus();
  const focus = vi.spyOn(HTMLElement.prototype, 'focus');
  await act(async () => { useTaskRunner.setState({ toolTask: proposal(3, 'proposal-2') }); });
  expect(document.activeElement).toBe(control);
  expect(focus).not.toHaveBeenCalled();
  expect(countdown().textContent).toMatch(/^3 /);
  await advance(3000);
  expect(approve().disabled).toBe(true);
  await view.setActive(false);
  await act(async () => { useTaskRunner.setState({ toolTask: proposal(7, 'proposal-3') }); });
  expect(document.activeElement).toBe(control);
  await view.setActive(true);
  expect(document.activeElement).toBe(control);
  expect(countdown().textContent).toMatch(/^7 /);
  expect(approve().disabled).toBe(false);
});

it.each(['hidden', 'covered'] as const)('receives a real store approval while %s without taking focus or cancelling the task', async mode => {
  const covered = mode === 'covered';
  const view = show(false, covered); await flush();
  const keepFocus = screen.getByRole('button', { name: covered ? 'Overlay control' : 'Keep my focus' });
  keepFocus.focus();
  const run = await start();
  await arrive(); await run.pending;
  expect(document.activeElement).toBe(keepFocus);
  expect(useTaskRunner.getState().toolTask?.proposal?.id).toBe('proposal-1');
  expect(useTaskRunner.getState().taskId).toBe(taskId);
  expect(useTaskRunner.getState().running).toBe(false);
  expect(view.controller.getSurfaceStatuses()['single-task'].running).toBe(true);
  expect(mocked.mock.calls.some(([command]) => command === 'cancel_workbench_tool_task')).toBe(false);
  // Keep a control present through the activity transition to detect unsolicited focus.
  const persistentControl = screen.getByRole('button', { name: 'Keep my focus' });
  persistentControl.focus();
  await view.setActive(true);
  expect(document.activeElement).toBe(persistentControl);
});

it.each(['hidden', 'covered'] as const)('measures countdown presentation commits while %s and stops after expiry', async mode => {
  const view = show(); await flush(); await start(); await arrive(proposal(12));
  view.commits.mockClear();
  await advance(3000);
  const activeCommits = view.commits.mock.calls.length;
  const activeText = countdown().textContent;
  await view.setActive(false, mode === 'covered');
  view.commits.mockClear();
  await advance(4000);
  const hiddenCommits = view.commits.mock.calls.length;
  const hiddenText = countdown().textContent;
  await view.setActive(true);
  expect(countdown().textContent).toMatch(/^5 /);
  await advance(5000);
  expect(approve().disabled).toBe(true);
  view.commits.mockClear();
  await advance(4000);
  const expiredCommits = view.commits.mock.calls.length;
  console.info('single-task approval presentation commits', { mode, activeCommits, hiddenCommits, expiredCommits });
  expect(activeCommits).toBeGreaterThan(0);
  expect.soft(hiddenCommits).toBe(0);
  expect.soft(hiddenText).toBe(activeText);
  expect.soft(expiredCommits).toBe(0);
});

it('uses the real deadline immediately on restoration and never focuses the old proposal again', async () => {
  const view = show(); await flush(); await start(); await arrive(proposal(5));
  const control = screen.getByRole('button', { name: 'Keep my focus' });
  control.focus();
  await view.setActive(false);
  // Sleep the presentation clock without running callbacks, as when a view is suspended.
  vi.setSystemTime(Date.now() + 6000);
  await view.setActive(true);
  expect.soft(document.activeElement).toBe(control);
  expect(countdown().textContent).toMatch(/^0 /);
  expect(approve().disabled).toBe(true);
  expect((screen.getByRole('button', { name: 'Reject and stop' }) as HTMLButtonElement).disabled).toBe(true);
  await act(async () => { await useTaskRunner.getState().decide('approve'); });
  expect(mocked.mock.calls.filter(([command, args]) => {
    const action = args?.action;
    return command === 'advance_workbench_tool_task' && action && typeof action === 'object' && 'type' in action && action.type === 'approve';
  })).toHaveLength(0);
});

it('preserves close confirmation and waits for hidden task termination through the real store', async () => {
  const view = show(false); await flush();
  const run = await start(); await arrive(); await run.pending;
  const confirm = vi.spyOn(Modal, 'confirm');
  let denied!: Promise<boolean>;
  await act(async () => { denied = view.controller.canLeave('single-task'); });
  expect(confirm).toHaveBeenCalledTimes(1);
  await act(async () => { await confirm.mock.calls[0][0].onCancel?.(); });
  expect(await denied).toBe(false);
  expect(useTaskRunner.getState().taskId).toBe(taskId);
  expect(mocked.mock.calls.some(([command]) => command === 'cancel_workbench_tool_task')).toBe(false);
  let closing!: Promise<boolean>;
  await act(async () => { closing = view.controller.canLeave('single-task'); });
  let settled = false;
  void closing.then(() => { settled = true; });
  await act(async () => { void confirm.mock.calls[1][0].onConfirm?.(); });
  expect(mocked).toHaveBeenCalledWith('cancel_workbench_tool_task', { taskId });
  expect(settled).toBe(false);
  await act(async () => { cancellation.resolve({ record: record('cancelled'), phase: 'finished' }); });
  expect(await closing).toBe(true);
  expect(useTaskRunner.getState().taskId).toBeNull();
  expect(screen.queryByRole('heading', { name: 'Review proposed tool call', hidden: true })).toBeNull();
});

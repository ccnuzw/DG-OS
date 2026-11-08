import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { TaskLaunch } from '../src/components/development/TaskLaunch';
import { useDevelopment, type Outcome } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { useAgentRunner, type AgentOpenInput } from '../src/stores/agentRunnerStore';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useAgentRunner.getState().reset(); useDevelopmentDrafts.getState().clear(); invoke.mockReset(); });
const project = { id: 'p', owner: 'u', name: 'Project', path: '/project', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: null };
const task = { id: 't', owner: 'u', project_id: 'p', goal: 'Goal', requirements: '', status: 'todo' as const, created_at: 1 };
const workspace = { id: 'w', project_id: 'p', path: '/actual-worktree', directory_identity: 'worktree', kind: 'original' };
const capability = { revision: 4, permission_epoch: 2, configuration: { engine: 'claude' as const, provider: { provider_id: 'provider', model_id: 'model', provider_revision: 0 } }, created_at: 1 };
const selection = { project_id: 'p', task_id: null, session_id: null };
function host() {
  invoke.mockImplementation(async (command, args) => {
    if (command === 'read_development_cli_provider') return capability;
    if (command === 'read_development_workspaces') return { workspaces: [workspace], selected_workspace_id: 'w', operation: null, operations: [] };
    if (command === 'save_development_task') return { ...task, goal: args.input.goal };
    if (command === 'read_development_workspace') return { projects: [project], tasks: [task], sessions: [], selection, operations: [] };
    return null;
  });
}
async function trust() {
  const checkbox = screen.getByRole('checkbox', { name: 'dev.trustProject' });
  await waitFor(() => expect((checkbox as HTMLButtonElement).disabled).toBe(false));
  fireEvent.click(checkbox);
  const provider = screen.getByRole('combobox', { name: 'capabilities.newSessionSource' });
  fireEvent.keyDown(provider, { key: 'ArrowDown' });
  fireEvent.click(await screen.findByRole('option', { name: 'capabilities.projectRevision' }));
}
it('creates one task/session, prevents double click, and retains IDs through acknowledged failure and manager return', async () => {
  host(); useDevelopment.setState({ projects: [project], tasks: [], selection });
  const pending = Promise.withResolvers<Outcome<null>>();
  const start = vi.fn<(input: AgentOpenInput, projectId: string, taskId: string) => Promise<Outcome<null>>>().mockReturnValue(pending.promise);
  const view = render(<TaskLaunch onStart={start} onContinue={vi.fn()} onManageProjects={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('dev.goal'), { target: { value: 'Goal' } });
  await trust(); fireEvent.click(screen.getByTestId('task-launch-start'));
  await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  const first = start.mock.calls[0][0];
  expect(first.intent).toBe('create');
  if (first.intent !== 'create') throw new Error('Expected a create request');
  expect(first.options.workspace).toBe('/actual-worktree');
  fireEvent.click(screen.getByTestId('task-launch-start'));
  expect(start).toHaveBeenCalledTimes(1);
  await act(async () => { pending.resolve({ ok: false, code: 'workspace_busy' }); });
  view.unmount();
  render(<TaskLaunch onStart={start} onContinue={vi.fn()} onManageProjects={vi.fn()} />);
  await trust(); fireEvent.click(screen.getByTestId('task-launch-start'));
  await waitFor(() => expect(start).toHaveBeenCalledTimes(2));
  const second = start.mock.calls[1][0];
  if (second.intent !== 'create') throw new Error('Expected a create request');
  expect(second.id).toBe(first.id);
  expect(second.task_id).toBe(first.task_id);
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_task')).toHaveLength(1);
});
it('never retries unknown task creation and reconciles explicit host task selection', async () => {
  host(); useDevelopment.setState({ projects: [project], tasks: [], selection });
  const base = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command, args) => { if (command === 'save_development_task') throw new Error('transport lost'); return base(command, args); });
  const start = vi.fn().mockResolvedValue({ ok: true, value: null });
  render(<TaskLaunch onStart={start} onContinue={vi.fn()} onManageProjects={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('dev.goal'), { target: { value: 'Goal' } }); await trust();
  fireEvent.click(screen.getByTestId('task-launch-start'));
  await screen.findByText('taskLaunch.taskOutcomeUnknown');
  expect((screen.getByTestId('task-launch-start') as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'taskLaunch.recheckTasks' }));
  fireEvent.click(await screen.findByRole('button', { name: 'Goal · t' }));
  await waitFor(() => expect(screen.queryByText('taskLaunch.taskOutcomeUnknown')).toBeNull());
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_task')).toHaveLength(1);
  expect(start).not.toHaveBeenCalled();
});
it('blocks unknown session restart while preserving the original ID for attach-only inspection', async () => {
  host(); useDevelopment.setState({ projects: [project], tasks: [task], selection: { ...selection, task_id: 't' } });
  const start = vi.fn<(input: AgentOpenInput, projectId: string, taskId: string) => Promise<Outcome<null>>>().mockResolvedValue({ ok: false, code: 'unknown' });
  const inspect = vi.fn().mockResolvedValue(undefined);
  render(<TaskLaunch initialTaskId="t" onStart={start} onContinue={vi.fn()} onInspectSession={inspect} onManageProjects={vi.fn()} />);
  await trust(); fireEvent.click(screen.getByTestId('task-launch-start'));
  await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  const opened = start.mock.calls[0][0];
  if (opened.intent !== 'create') throw new Error('Expected a create request');
  const id = opened.id;
  await waitFor(() => expect((screen.getByTestId('task-launch-start') as HTMLButtonElement).disabled).toBe(true));
  fireEvent.click(screen.getByRole('button', { name: `taskLaunch.inspectSession · ${id}` }));
  expect(inspect).toHaveBeenCalledWith(id);
  expect(start).toHaveBeenCalledTimes(1);
});
it('fails stale permission epochs before starting and keeps a selected project revision', async () => {
  host(); useDevelopment.setState({ projects: [project], tasks: [task], selection: { ...selection, task_id: 't' } });
  const base = invoke.getMockImplementation()!;
  let reads = 0;
  invoke.mockImplementation(async (command, args) => command === 'read_development_cli_provider' ? { ...capability, permission_epoch: ++reads === 1 ? 2 : 3 } : base(command, args));
  const start = vi.fn().mockResolvedValue({ ok: true, value: null });
  render(<TaskLaunch initialTaskId="t" requestedRevision={4} onStart={start} onContinue={vi.fn()} onManageProjects={vi.fn()} />);
  await trust(); fireEvent.click(screen.getByTestId('task-launch-start'));
  await waitFor(() => expect(screen.getByRole('alert').textContent).toContain('dev.error.capability_conflict'));
  expect(start).not.toHaveBeenCalled();
});
it('connects live sessions without trust and requires actual-directory trust for saved native restart', async () => {
  host(); useDevelopment.setState({ projects: [project], tasks: [task], sessions: [{ id: 's', owner: 'u', engine: 'codex', native_id: 'native', workspace: workspace.path, executable: null, model: null, project_id: 'p', task_id: 't' }], selection: { ...selection, task_id: 't' } });
  const resume = vi.fn().mockResolvedValue(undefined);
  render(<TaskLaunch initialTaskId="t" onStart={vi.fn().mockResolvedValue({ ok: true, value: null })} onContinue={resume} onManageProjects={vi.fn()} />);
  expect((screen.getByTestId('task-launch-continue-s') as HTMLButtonElement).disabled).toBe(true);
  await trust(); fireEvent.click(screen.getByTestId('task-launch-continue-s'));
  await waitFor(() => expect(resume).toHaveBeenCalledWith(expect.objectContaining({ id: 's' }), true));
  await act(async () => { useAgentRunner.setState({ live: { s: 'idle' } }); });
  fireEvent.click(screen.getByRole('checkbox', { name: 'dev.trustProject' }));
  expect((screen.getByTestId('task-launch-continue-s') as HTMLButtonElement).disabled).toBe(false);
});

it('keeps successful task/session identity and never replays success on a later click', async () => {
  host(); useDevelopment.setState({ projects: [project], tasks: [], selection });
  const start = vi.fn<(input: AgentOpenInput, projectId: string, taskId: string) => Promise<Outcome<null>>>().mockResolvedValue({ ok: true, value: null });
  const view = render(<TaskLaunch onStart={start} onContinue={vi.fn()} onManageProjects={vi.fn()} />);
  fireEvent.change(screen.getByLabelText('dev.goal'), { target: { value: 'Goal' } });
  await trust(); fireEvent.click(screen.getByTestId('task-launch-start'));
  await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
  await waitFor(() => expect((screen.getByTestId('task-launch-start') as HTMLButtonElement).disabled).toBe(true));
  fireEvent.click(screen.getByTestId('task-launch-start'));
  expect(start).toHaveBeenCalledTimes(1);
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_task')).toHaveLength(1);
  await waitFor(() => expect(useDevelopmentDrafts.getState().entries).toEqual({}));
  view.unmount();
  render(<TaskLaunch initialTaskId={null} onStart={start} onContinue={vi.fn()} onManageProjects={vi.fn()} />);
  expect((screen.getByLabelText('dev.goal') as HTMLInputElement).value).toBe('');
});

it.each(['cli_incompatible', 'provider_revision_changed', 'provider_binding_unconfirmed', 'session_busy', 'bridge_unavailable', 'cleanup_pending', 'session_exists', 'unrecognized transport failure', 'result_unknown'].flatMap(failure => (['outcome', 'exception'] as const).map(mode => [failure, mode] as const)))('keeps definite launch rejections actionable and never replays uncertainty (%s, %s)', async (failure, mode) => {
 host();
 useDevelopment.setState({ projects: [project], tasks: [task], selection: { ...selection, task_id: 't' } });
 const base = invoke.getMockImplementation()!;
 invoke.mockImplementation(async (command, args) => {
  if (command === 'open_agent_session') throw failure;
  return base(command, args);
 });
 const onStart = async (input: AgentOpenInput): Promise<Outcome<null>> => {
  if (mode === 'exception') { await invoke('open_agent_session', { input }); return { ok: true, value: null }; }
  await useAgentRunner.getState().open(input);
  const code = useAgentRunner.getState().error;
  return code ? { ok: false, code } : { ok: true, value: null };
 };
 render(<TaskLaunch initialTaskId="t" onStart={onStart} onContinue={vi.fn()} onInspectSession={vi.fn()} onManageProjects={vi.fn()} />);
 await trust();
 fireEvent.click(screen.getByTestId('task-launch-start'));
 const unknown = failure === 'unrecognized transport failure' || failure === 'result_unknown';
 const blocked = unknown || failure === 'cleanup_pending' || failure === 'session_exists';
 await waitFor(() => expect(screen.getByRole('alert').textContent).toBe(`dev.error.${unknown ? 'unknown' : failure}`));
 const requests = invoke.mock.calls.filter(([command]) => command === 'open_agent_session');
 expect(requests).toHaveLength(1);
 expect(screen.getByTestId('task-launch-start').hasAttribute('disabled')).toBe(blocked);
 fireEvent.click(screen.getByTestId('task-launch-start'));
 await act(async () => {});
 const after = invoke.mock.calls.filter(([command]) => command === 'open_agent_session');
 expect(after).toHaveLength(blocked ? 1 : 2);
 if (!blocked) expect(after[1][1].input.id).toBe(requests[0][1].input.id);
 expect(invoke.mock.calls.filter(([command]) => command === 'save_development_task')).toHaveLength(0);
});

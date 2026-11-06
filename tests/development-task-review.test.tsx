import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { TaskReview } from '../src/components/development/TaskReview';
import { useDevelopment, type DevelopmentTask, type Project } from '../src/stores/developmentStore';
import { useDevelopmentDrafts, developmentDraftKey } from '../src/stores/developmentDrafts';
import { useDevelopmentRuntime, type DevelopmentProcess } from '../src/stores/developmentRuntime';
import { chooseContext } from '../src/stores/developmentSelection';
import type { DevReview } from '../src/stores/developmentReview';
import type { GitReview, WorkspaceView } from '../src/stores/developmentDelivery';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));

const project: Project = { id: 'project', owner: 'owner', name: 'Project', path: '/project', directory_identity: 'directory', archived: false, created_at: 1, last_opened_at: null };
let task: DevelopmentTask;
let review: DevReview;
let git: GitReview;
let workspaces: WorkspaceView;
let processes: DevelopmentProcess[];
let failSave: boolean;
const callbacks = { onOpenDiff: vi.fn(), onOpenRuntime: vi.fn(), onOpenDelivery: vi.fn() };
function process(overrides: Partial<DevelopmentProcess> = {}): DevelopmentProcess {
  return { id: 'verify', project_id: project.id, task_id: 'task', kind: 'command', source: 'host_command', status: 'exited', output: '', truncated: false,
    exit_code: 0, reason: null, started_at: 3, finished_at: 4, program: 'node', args: ['--test'], cwd: '', fingerprint: 'current', start_fingerprint: 'current',
    verification_status: 'observed_stable', verification_complete: true, mutation_observed: false, preview_url: null, ...overrides };
}
beforeEach(() => {
  task = { id: 'task', owner: project.owner, project_id: project.id, goal: 'Review task', requirements: 'Keep original requirements', status: 'review', created_at: 1 };
  review = { task_id: task.id, captured_at: 2, baseline_late: false, truncated: false, fingerprint: 'current', changes: [{ path: 'code.ts', kind: 'modified', pre_existing: false, origin: 'uncertain', previewable: true }], runs_stale: false };
  git = { task_id: task.id, kind: 'git', path: project.path, branch: 'main', head: 'head', merge_heads: [], files: [], index: [], conflicts: [], staged_diff: '', worktree_diff: '', review_token: 'git-current', baseline_version: 'baseline', initial_staged_paths: [], file_versions: {} };
  workspaces = { workspaces: [], selected_workspace_id: null, operation: null, operations: [] };
  processes = [];
  failSave = false;
  useDevelopment.setState({ projects: [project], tasks: [task], sessions: [], selection: { project_id: project.id, task_id: task.id, session_id: null }, operations: [], loaded: true });
  invoke.mockImplementation(async (command: string, args?: { input?: Partial<DevelopmentTask> }) => {
    if (command === 'read_development_workspace') return { projects: [project], tasks: [task], sessions: [], operations: [], selection: { project_id: project.id, task_id: task.id, session_id: null } };
    if (command === 'read_development_review') return structuredClone(review);
    if (command === 'list_development_runs') return [];
    if (command === 'read_development_git') return structuredClone(git);
    if (command === 'read_development_workspaces') return structuredClone(workspaces);
    if (command === 'list_development_processes') return structuredClone(processes);
    if (command === 'save_development_task') { if (failSave) throw 'storage_failed'; task = { ...task, ...args?.input }; return task; }
    if (command === 'select_development_context') return null;
    throw new Error(`Unexpected command: ${command}`);
  });
});
afterEach(() => {
  cleanup(); useDevelopment.getState().reset(); useDevelopmentRuntime.getState().reset();
  useDevelopmentDrafts.getState().clear(); invoke.mockReset(); vi.clearAllMocks(); localStorage.clear();
});
async function mountReview() {
  const view = render(<TaskReview projectId={project.id} taskId={task.id} {...callbacks} />);
  await waitFor(() => expect(screen.getByTestId('dev-task-review-verification')).toBeTruthy());
  return view;
}
function confirm(accept = false, explanation = '') {
  if (accept) fireEvent.click(screen.getByTestId('dev-task-review-accept'));
  if (explanation) fireEvent.change(screen.getByTestId('dev-task-review-explanation'), { target: { value: explanation } });
  fireEvent.click(screen.getByTestId('dev-task-review-confirm'));
}

it('requires both explicit acceptance and an explanation and persists their provenance without Git dispatch', async () => {
  await mountReview();
  const complete = screen.getByTestId('dev-task-review-complete') as HTMLButtonElement;
  confirm(true);
  expect(complete.disabled).toBe(true);
  fireEvent.change(screen.getByTestId('dev-task-review-explanation'), { target: { value: 'Manual review is sufficient for this documentation-only task.' } });
  fireEvent.click(screen.getByTestId('dev-task-review-accept'));
  expect(complete.disabled).toBe(true);
  fireEvent.click(screen.getByTestId('dev-task-review-accept'));
  fireEvent.click(complete);
  await waitFor(() => expect(task.status).toBe('completed'));
  expect(task.requirements.startsWith('Keep original requirements\n\n[DG-OS task review acceptance]')).toBe(true);
  expect(task.requirements).toContain('Verification: not_requested');
  expect(task.requirements).toContain('Current fingerprint: current');
  expect(task.requirements).toContain('Manual review is sufficient');
  expect(invoke.mock.calls.some(([command]) => ['execute_development_git', 'start_development_process'].includes(command))).toBe(false);
  expect(invoke.mock.calls.filter(([command]) => command === 'read_development_review')).toHaveLength(2);
});

it.each([
  { overrides: { verification_status: 'stale' }, state: 'stale' },
  { overrides: { fingerprint: 'old', start_fingerprint: 'old' }, state: 'stale' },
  { overrides: { verification_status: 'failed', exit_code: 1 }, state: 'failed' },
  { overrides: { verification_complete: false }, state: 'incomplete' },
  { overrides: { mutation_observed: true }, state: 'changed' },
])('does not treat $state verification as a pass', async ({ overrides, state }) => {
  processes = [process(overrides)];
  await mountReview();
  expect(screen.getByTestId('dev-task-review-verification').getAttribute('data-verification')).toBe(state);
  confirm();
  expect((screen.getByTestId('dev-task-review-complete') as HTMLButtonElement).disabled).toBe(true);
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_task')).toBe(false);
});

it('does not use an older successful verification to hide the latest failed verification', async () => {
  processes = [process({ id: 'old', started_at: 1 }), process({ id: 'latest', started_at: 5, exit_code: 1, verification_status: 'failed' })];
  await mountReview();
  expect(screen.getByTestId('dev-task-review-verification').getAttribute('data-verification')).toBe('failed');
});

it('completes a directory task with fresh passing verification without editing requirements or invoking Git', async () => {
  git = { ...git, kind: 'directory', branch: null, head: null };
  processes = [process()];
  await mountReview();
  expect(screen.queryByTestId('dev-task-review-accept')).toBeNull();
  confirm(); fireEvent.click(screen.getByTestId('dev-task-review-complete'));
  await waitFor(() => expect(task.status).toBe('completed'));
  expect(task.requirements).toBe('Keep original requirements');
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_task')).toHaveLength(1);
  expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git')).toBe(false);
});

it('blocks unknown Git operations and routes to existing delivery recovery without replay', async () => {
  workspaces.operations = [{ id: 'fixed-operation', kind: 'git_push', status: 'unknown', project_id: project.id, task_id: task.id, workspace_id: null, request_fingerprint: 'request', details: {} }];
  await mountReview(); confirm(true, 'Accept missing verification');
  expect((screen.getByTestId('dev-task-review-complete') as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'taskReview.reconcileDelivery' }));
  expect(callbacks.onOpenDelivery).toHaveBeenCalledOnce();
  expect(invoke.mock.calls.some(([command]) => ['execute_development_git', 'save_development_task'].includes(command))).toBe(false);
});

it('blocks unresolved durable request identities even when the host receipt has not arrived', async () => {
  localStorage.setItem(`dgos.delivery.requests:${project.owner}`, JSON.stringify([{ id: 'fixed-request', kind: 'git_push', project_id: project.id, task_id: task.id }]));
  await mountReview(); confirm(true, 'Accept missing verification');
  expect((screen.getByTestId('dev-task-review-complete') as HTMLButtonElement).disabled).toBe(true);
});

it('blocks unknown runtime operations and conflicts even when missing verification is accepted', async () => {
  useDevelopmentRuntime.setState({ operations: [{ id: 'fixed-runtime', epoch: useDevelopment.getState().epoch, projectId: project.id, taskId: task.id, action: 'start', targetId: 'original-process', phase: 'unknown' }] });
  git.conflicts = [{ path: 'code.ts', stage: 2, mode: '100644', oid: 'conflict' }];
  await mountReview(); confirm(true, 'Accept missing verification');
  expect((screen.getByTestId('dev-task-review-complete') as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(screen.getByRole('button', { name: 'taskReview.reconcileRuntime' }));
  expect(callbacks.onOpenRuntime).toHaveBeenCalledOnce();
  expect(invoke.mock.calls.some(([command]) => ['start_development_process', 'save_development_task'].includes(command))).toBe(false);
});

it('re-reads before completion and requires confirmation again when the files changed', async () => {
  processes = [process()];
  await mountReview(); confirm();
  review = { ...review, fingerprint: 'new-files' };
  fireEvent.click(screen.getByTestId('dev-task-review-complete'));
  await waitFor(() => expect(screen.getByTestId('dev-task-review-verification').getAttribute('data-verification')).toBe('stale'));
  expect((screen.getByTestId('dev-task-review-confirm') as HTMLInputElement).checked).toBe(false);
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_task')).toBe(false);
});

it('preserves newer host requirements and refuses completion against the earlier task snapshot', async () => {
  await mountReview(); confirm(true, 'Accept missing verification');
  task = { ...task, requirements: 'New requirements from another view' };
  fireEvent.click(screen.getByTestId('dev-task-review-complete'));
  await waitFor(() => expect((screen.getByTestId('dev-task-review-confirm') as HTMLInputElement).checked).toBe(false));
  expect(task.requirements).toBe('New requirements from another view');
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_task')).toBe(false);
});

it('preserves task edits and review explanations across a failed save and a panel remount', async () => {
  failSave = true;
  const view = await mountReview(); confirm(true, 'An explicit review acceptance explanation');
  fireEvent.click(screen.getByTestId('dev-task-review-complete'));
  await screen.findByRole('alert');
  view.unmount();
  await mountReview();
  expect((screen.getByTestId('dev-task-review-explanation') as HTMLTextAreaElement).value).toBe('An explicit review acceptance explanation');
  expect((screen.getByTestId('dev-task-review-accept') as HTMLInputElement).checked).toBe(false);
  act(() => useDevelopmentDrafts.setState({ entries: { ...useDevelopmentDrafts.getState().entries, [developmentDraftKey('task', project.id, task.id)]: { kind: 'task', value: { requirements: 'Unsaved task edit' }, baseline: { requirements: task.requirements } } } }));
  confirm(true);
  expect((screen.getByTestId('dev-task-review-complete') as HTMLButtonElement).disabled).toBe(true);
});

it.each(['context', 'account'])('does not save after a %s change while fresh completion reads are pending', async mode => {
  processes = [process()];
  await mountReview(); confirm();
  const pending = Promise.withResolvers<DevReview>();
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args: unknown) => command === 'read_development_review' ? pending.promise : original(command, args));
  fireEvent.click(screen.getByTestId('dev-task-review-complete'));
  await act(async () => {
    if (mode === 'account') useDevelopment.getState().reset();
    else await chooseContext({ project_id: 'other-project', task_id: 'other-task', session_id: null });
    pending.resolve(review);
  });
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_task')).toBe(false);
});

it('routes changes and revalidation to the existing surfaces without executing commands', async () => {
  await mountReview();
  fireEvent.click(screen.getByRole('button', { name: /code.ts/ }));
  fireEvent.click(screen.getByRole('button', { name: 'taskReview.revalidate' }));
  expect(callbacks.onOpenDiff).toHaveBeenCalledWith('code.ts');
  expect(callbacks.onOpenRuntime).toHaveBeenCalledOnce();
  expect(invoke.mock.calls.some(([command]) => command === 'start_development_process')).toBe(false);
});

it('never lets a simultaneous passing record hide a failed verification', async () => {
  processes = [process({ id: 'pass' }), process({ id: 'failed', exit_code: 1, verification_status: 'failed' })];
  await mountReview();
  expect(screen.getByTestId('dev-task-review-verification').getAttribute('data-verification')).toBe('failed');
});

it('blocks when a conflict appears in the fresh host snapshot after confirmation', async () => {
  processes = [process()];
  await mountReview(); confirm();
  git = { ...git, conflicts: [{ path: 'new-conflict.ts', stage: 2, mode: '100644', oid: 'conflict' }] };
  fireEvent.click(screen.getByTestId('dev-task-review-complete'));
  await screen.findByText('new-conflict.ts');
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_task')).toBe(false);
});

it('saves the existing review enum without accepting verification or delivering Git', async () => {
  task = { ...task, status: 'in_progress' };
  await mountReview();
  fireEvent.click(screen.getByRole('button', { name: 'taskReview.markReview' }));
  await waitFor(() => expect(task.status).toBe('review'));
  expect(task.requirements).toBe('Keep original requirements');
  expect(invoke.mock.calls.some(([command]) => ['execute_development_git', 'start_development_process'].includes(command))).toBe(false);
});

it.each(['stale', 'failed'])('records explicit acceptance of %s verification as that state', async state => {
  processes = [process({ verification_status: state, exit_code: state === 'failed' ? 1 : 0 })];
  await mountReview(); confirm(true, 'Reviewed the limitation and accept completion.');
  fireEvent.click(screen.getByTestId('dev-task-review-complete'));
  await waitFor(() => expect(task.status).toBe('completed'));
  expect(task.requirements).toContain(`Verification: ${state}`);
  expect(task.requirements).toContain('Verification process: verify');
  expect(task.requirements).toContain('Reviewed the limitation and accept completion.');
});

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { Modal } from '@dgos/ui';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { DeliveryPanel } from '../src/components/development/DeliveryPanel';
import { useDevelopment } from '../src/stores/developmentStore';
import { deliveryRequests, type GitOperation, type GitReview } from '../src/stores/developmentDelivery';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key }));
const project = { id: 'p', owner: 'alice', name: 'Project', path: '/original', directory_identity: 'dir', archived: false, created_at: 1, last_opened_at: null };
const receipt = (id: string, status: GitOperation['status'] = 'unknown', kind = 'git_stage'): GitOperation => ({ id, status, kind, project_id: 'p', task_id: 't', workspace_id: null, request_fingerprint: 'fingerprint', details: {} });
let receipts: GitOperation[];
let mergeHeads: string[];
const workspace = () => ({ workspaces: [{ id: 'w', path: '/task-worktree', kind: 'managed' }], selected_workspace_id: 'w', operation: null, operations: receipts });
const props = { projectId: 'p', taskId: 't', sessions: [], live: {}, capabilityRevision: 0, workspacePath: '/task-worktree' };
async function show() { const view = render(<DeliveryPanel {...props} />); await act(async () => {}); return view; }
beforeEach(() => {
 localStorage.clear(); useDevelopment.getState().reset(); useDevelopment.setState({ projects: [project] }); receipts = []; mergeHeads = [];
 invoke.mockImplementation(async (command: string) => {
  if (command === 'read_development_workspaces') return workspace();
  if (command === 'read_development_git') return { task_id: 't', kind: 'git', path: '/task-worktree', files: [{ path: 'code.txt', index: ' ', worktree: 'M' }], conflicts: [], initial_staged_paths: [], staged_diff: '', worktree_diff: 'changed', merge_heads: mergeHeads, file_versions: {}, baseline_version: '', review_token: 'review' };
  if (command === 'list_development_handoffs') return [];
  if (command === 'read_development_workspace') return { projects: [project], tasks: [], sessions: [], operations: receipts, selection: { project_id: 'p', task_id: 't', session_id: null } };
  return null;
 });
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); localStorage.clear(); invoke.mockReset(); });
it('persists identity before IPC, survives disposal and receipt loss, then keeps reconciliation on reentry', async () => {
 const original = invoke.getMockImplementation()!;
 const deferred = Promise.withResolvers<unknown>(); let id = '';
 invoke.mockImplementation(async (command: string, args: any) => {
  if (command === 'execute_development_git') {
   id = args.input.id;
   expect(deliveryRequests('alice')[0].id).toBe(id);
   receipts = [receipt(id)]; return deferred.promise;
  }
  if (command === 'reconcile_development_git') { receipts = [receipt(id, 'succeeded')]; return receipts[0]; }
  return original(command, args);
 });
 const first = await show(); fireEvent.click(screen.getByRole('checkbox', { name: /code.txt/ })); fireEvent.click(screen.getByRole('button', { name: 'delivery.stage' }));
 first.unmount(); await act(async () => { deferred.reject('IPC disconnected'); });
 const second = await show();
 expect(screen.getByRole('status').textContent).toContain('delivery.operation.unknown');
 expect(screen.getByRole('button', { name: new RegExp(id) })).toBeTruthy();
 expect(useDevelopment.getState().operations.find(item => item.id === id)).toMatchObject({ status: 'unknown', project_id: 'p', task_id: 't' });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.reconcile' })); await act(async () => {});
 expect(useDevelopment.getState().operations[0].status).toBe('succeeded'); second.unmount();
 await show();
 expect(deliveryRequests('alice')).toEqual([]);
 expect(invoke.mock.calls.filter(([command]) => command === 'execute_development_git')).toHaveLength(1);
});
it('restores workspace receipts and isolates unknown identities by account and task', async () => {
 const id = crypto.randomUUID(); localStorage.setItem('dgos.delivery.requests:alice', JSON.stringify([{ id, kind: 'workspace_create', project_id: 'p', task_id: 't' }]));
 const view = await show();
 expect(screen.getByRole('status').textContent).toContain('delivery.operation.unknown');
 expect(screen.getByRole('button', { name: new RegExp(id) })).toBeTruthy();
 expect(deliveryRequests('alice')).toEqual([{ id, kind: 'workspace_create', project_id: 'p', task_id: 't' }]);
 expect((screen.getByRole('button', { name: 'delivery.stage' }) as HTMLButtonElement).disabled).toBe(true);
 view.rerender(<DeliveryPanel {...props} taskId="other" />); await act(async () => {});
 fireEvent.click(screen.getByRole('checkbox', { name: /code.txt/ }));
 expect((screen.getByRole('button', { name: 'delivery.stage' }) as HTMLButtonElement).disabled).toBe(false);
 await act(async () => { useDevelopment.getState().reset(); useDevelopment.setState({ projects: [{ ...project, owner: 'bob' }] }); });
 view.rerender(<DeliveryPanel {...props} />); await act(async () => {});
 fireEvent.click(screen.getByRole('checkbox', { name: /code.txt/ }));
 expect((screen.getByRole('button', { name: 'delivery.stage' }) as HTMLButtonElement).disabled).toBe(false);
});
it('permits full-index confirmed empty-diff merges and blocks ordinary empty commits', async () => {
 const view = await show(); fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'finish merge' } }); fireEvent.click(screen.getByRole('checkbox', { name: 'delivery.confirmAllIndex' }));
 expect((screen.getByRole('button', { name: 'delivery.commit' }) as HTMLButtonElement).disabled).toBe(true);
 view.unmount(); mergeHeads = ['topic-head']; await show(); fireEvent.click(screen.getByRole('checkbox', { name: 'delivery.confirmAllIndex' }));
 expect((screen.getByRole('button', { name: 'delivery.commit' }) as HTMLButtonElement).disabled).toBe(false);
});

function gitReview(overrides: Partial<GitReview> = {}): GitReview {
 return { task_id: 't', kind: 'git', path: '/task-worktree', branch: 'main', head: 'a'.repeat(40), merge_heads: [], files: [{ path: 'code.txt', index: ' ', worktree: 'M' }], index: [], conflicts: [], staged_diff: '', worktree_diff: 'changed', review_token: 'review', baseline_version: '', initial_staged_paths: [], file_versions: {}, ...overrides };
}

it('uses the same Git engine in compact mode without workspace, handoff or push controls', async () => {
 const onOpenDiff = vi.fn();
 render(<DeliveryPanel {...props} mode="git" onOpenDiff={onOpenDiff} />); await act(async () => {});
 for (const label of ['delivery.workspaces', 'delivery.handoffs']) expect(screen.queryByRole('region', { name: label })).toBeNull();
 for (const label of ['delivery.create', 'delivery.import', 'delivery.push', 'delivery.switch', 'delivery.merge', 'delivery.prepareHandoff']) expect(screen.queryByRole('button', { name: label })).toBeNull();
 for (const label of ['delivery.stage', 'delivery.unstage', 'delivery.commit', 'delivery.openGitHistory', 'delivery.previewPull']) expect(screen.getByRole('button', { name: label })).toBeTruthy();
 expect(screen.getByText('workbenchGit.workspaceScope')).toBeTruthy();
 expect(screen.getByText('workbenchGit.noTaskBaseline')).toBeTruthy();
 fireEvent.click(screen.getByRole('button', { name: 'code.txt' }));
 expect(onOpenDiff).toHaveBeenCalledWith('code.txt');
 expect((screen.getByRole('checkbox', { name: 'code.txt' }) as HTMLInputElement).checked).toBe(false);
 expect(invoke.mock.calls.some(([command]) => command === 'list_development_handoffs' || command === 'execute_development_git' || command === 'preview_development_git_pull' || command === 'read_development_git_history')).toBe(false);
});

it('refreshes native completion without losing valid path selection or commit drafts or repeating actions', async () => {
 const original = invoke.getMockImplementation()!;
 let current = gitReview({ files: [{ path: 'code.txt', index: ' ', worktree: 'M' }, { path: 'gone.txt', index: ' ', worktree: 'M' }], staged_diff: 'staged' });
 invoke.mockImplementation(async (command: string, args: unknown) => command === 'read_development_git' ? current : original(command, args));
 const view = render(<DeliveryPanel {...props} mode="git" refreshKey={0} />); await act(async () => {});
 fireEvent.click(screen.getByRole('checkbox', { name: 'code.txt' })); fireEvent.click(screen.getByRole('checkbox', { name: 'gone.txt' }));
 fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Keep this commit draft' } });
 fireEvent.click(screen.getByRole('checkbox', { name: 'delivery.confirmAllIndex' }));
 current = gitReview({ review_token: 'after-native-turn', staged_diff: 'staged' });
 view.rerender(<DeliveryPanel {...props} mode="git" refreshKey={1} />); await act(async () => {});
 expect((screen.getByRole('checkbox', { name: 'code.txt' }) as HTMLInputElement).checked).toBe(true);
 expect(screen.queryByRole('checkbox', { name: 'gone.txt' })).toBeNull();
 expect((screen.getByLabelText('delivery.commitMessage') as HTMLInputElement).value).toBe('Keep this commit draft');
 expect((screen.getByRole('checkbox', { name: 'delivery.confirmAllIndex' }) as HTMLInputElement).checked).toBe(false);
 expect(invoke.mock.calls.filter(([command]) => command === 'read_development_git')).toHaveLength(2);
 expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git')).toBe(false);
});

it('keeps native refresh read-only and queued behind an in-flight explicit stage', async () => {
 const original = invoke.getMockImplementation()!; const deferred = Promise.withResolvers<unknown>(); let id = '';
 invoke.mockImplementation(async (command: string, args: { input: { id: string } }) => {
  if (command === 'execute_development_git') { id = args.input.id; return deferred.promise; }
  return original(command, args);
 });
 const view = render(<DeliveryPanel {...props} mode="git" refreshKey={0} />); await act(async () => {});
 fireEvent.click(screen.getByRole('checkbox', { name: 'code.txt' })); fireEvent.click(screen.getByRole('button', { name: 'delivery.stage' }));
 view.rerender(<DeliveryPanel {...props} mode="git" refreshKey={1} />); await act(async () => {});
 expect(invoke.mock.calls.filter(([command]) => command === 'read_development_git')).toHaveLength(1);
 expect((screen.getByRole('button', { name: 'delivery.stage' }) as HTMLButtonElement).disabled).toBe(true);
 await act(async () => { receipts = [receipt(id, 'succeeded')]; deferred.resolve(receipts[0]); });
 expect(invoke.mock.calls.filter(([command]) => command === 'execute_development_git')).toHaveLength(1);
 expect((screen.getByRole('button', { name: 'delivery.stage' }) as HTMLButtonElement).disabled).toBe(false);
 expect(invoke.mock.calls.filter(([command]) => command === 'read_development_git').length).toBeGreaterThan(1);
});

it('keeps guarded restore confirmation and sends only explicit hashed paths after approval', async () => {
 const original = invoke.getMockImplementation()!;
 invoke.mockImplementation(async (command: string, args: { input: { id: string } }) => {
  if (command === 'read_development_git') return gitReview({ baseline_version: 'baseline', file_versions: { 'code.txt': 'hash:metadata' } });
  if (command === 'execute_development_git') { receipts = [receipt(args.input.id, 'succeeded', 'git_discard')]; return receipts[0]; }
  return original(command, args);
 });
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
 try {
  render(<DeliveryPanel {...props} mode="git" />); await act(async () => {});
  expect(screen.getByText('workbenchGit.taskBaseline')).toBeTruthy();
  fireEvent.click(screen.getByRole('checkbox', { name: 'code.txt' })); fireEvent.click(screen.getByRole('button', { name: 'delivery.discard' })); await act(async () => {});
  expect(confirm.mock.calls[0][0].content).toContain('code.txt');
  expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git')).toBe(false);
  confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  fireEvent.click(screen.getByRole('button', { name: 'delivery.discard' })); await act(async () => {});
  expect(invoke.mock.calls.find(([command]) => command === 'execute_development_git')?.[1].input.action).toEqual({ kind: 'discard', baseline_version: 'baseline', files: [{ path: 'code.txt', expected_hash: 'hash' }], confirmed: true });
 } finally { confirm.mockRestore(); }
});

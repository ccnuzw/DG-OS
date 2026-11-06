import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { DeliveryPanel } from '../src/components/development/DeliveryPanel';
import { useDevelopment } from '../src/stores/developmentStore';
import { deliveryRequests, type GitOperation } from '../src/stores/developmentDelivery';
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

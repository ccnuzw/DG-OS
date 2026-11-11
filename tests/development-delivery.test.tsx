import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { Modal } from '@dgos/ui';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { DeliveryPanel } from '../src/components/development/DeliveryPanel';
import { useDevelopment } from '../src/stores/developmentStore';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { deliveryRequests, type GitOperation, type StoredHandoff } from '../src/stores/developmentDelivery';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
const project = { id: 'p', owner: 'alice', name: 'Project', path: '/original', directory_identity: 'dir', archived: false, created_at: 1, last_opened_at: null };
const receipt = (id: string, status: GitOperation['status'] = 'unknown', kind = 'workspace_create'): GitOperation => ({ id, status, kind, project_id: 'p', task_id: 't', workspace_id: null, request_fingerprint: 'fingerprint', details: {} });
const prepared: StoredHandoff = { id: 'handoff', owner: 'alice', project_id: 'p', task_id: 't', source_session_id: 'source', target_engine: 'claude', permission_epoch: 0, summary: 'Prepared summary', fingerprint: 'fingerprint', created_at: 1 };
let receipts: GitOperation[];
const workspace = () => ({ workspaces: [{ id: 'w', path: '/task-worktree', kind: 'managed' }], selected_workspace_id: 'w', operation: null, operations: receipts });
const props = { projectId: 'p', taskId: 't', sessions: [{ id: 'source', engine: 'codex' as const }, { id: 'other', engine: 'claude' as const }] };
async function show(extra: Partial<Parameters<typeof DeliveryPanel>[0]> = {}) { const view = render(<DeliveryPanel {...props} {...extra} />); await act(async () => {}); return view; }
beforeEach(() => {
 localStorage.clear(); useDevelopment.getState().reset(); useDevelopment.setState({ projects: [project] }); receipts = [];
 invoke.mockImplementation(async (command: string) => {
  if (command === 'read_development_workspaces') return workspace();
  if (command === 'list_development_handoffs') return [prepared, { ...prepared, id: 'accepted', target_session_id: 'target' }];
  if (command === 'read_development_workspace') return { projects: [project], tasks: [], sessions: [], operations: receipts, selection: { project_id: 'p', task_id: 't', session_id: null } };
  throw new Error(`Unexpected command ${command}`);
 });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); useDevelopment.getState().reset(); localStorage.clear(); invoke.mockReset(); });
const executionCalls = () => invoke.mock.calls.filter(([command]) => ['execute_development_git', 'prepare_development_handoff', 'accept_development_handoff', 'send_agent_message', 'create_agent_session'].includes(command));
it('routes Git and model continuation to the parent without creating, sending or writing', async () => {
 const onOpenGit = vi.fn(), onContinueModel = vi.fn();
 await show({ onOpenGit, onContinueModel });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.openGit' }));
 fireEvent.change(screen.getByLabelText('delivery.sourceSession'), { target: { value: 'source' } });
 fireEvent.change(screen.getByLabelText('delivery.remaining'), { target: { value: 'Continue this work' } });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.continueModel' }));
 expect(onOpenGit).toHaveBeenCalledExactlyOnceWith();
 expect(onContinueModel).toHaveBeenCalledExactlyOnceWith('source');
 expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('delivery', 'p', 't')].value).toEqual({ sourceId: 'source', remaining: 'Continue this work' });
 expect(screen.queryByLabelText('delivery.commitMessage')).toBeNull();
 expect(executionCalls()).toEqual([]);
});
it('keeps prepared history and edited summary bound to its source instead of the selected runtime', async () => {
 const onContinueModel = vi.fn(); await show({ onContinueModel });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.useDraft', hidden: true }));
 fireEvent.change(screen.getByLabelText('delivery.summary'), { target: { value: 'User-edited unsent summary' } });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.continueSummary' }));
 expect(onContinueModel).toHaveBeenCalledExactlyOnceWith('source', 'handoff');
 expect(useDevelopmentDrafts.getState().entries[developmentDraftKey('handoff', 'p', 't', 'handoff')].value).toBe('User-edited unsent summary');
 expect(screen.getByText(/delivery.accepted/)).toBeTruthy();
 fireEvent.change(screen.getByLabelText('delivery.sourceSession'), { target: { value: 'other' } });
 expect(screen.queryByLabelText('delivery.summary')).toBeNull();
 fireEvent.click(screen.getByRole('button', { name: 'delivery.continueModel' }));
 expect(onContinueModel).toHaveBeenLastCalledWith('other');
 expect(executionCalls()).toEqual([]);
});
it('preserves worktree request identity across lost receipts and reconciles only the original id', async () => {
 const original = invoke.getMockImplementation()!, pending = Promise.withResolvers<unknown>(); let id = '';
 invoke.mockImplementation(async (command: string, args: { input: { request_id: string; id: string } }) => {
  if (command === 'save_development_workspaces') { id = args.input.request_id; expect(deliveryRequests('alice')[0].id).toBe(id); return pending.promise; }
  if (command === 'reconcile_development_git') { expect(args.input.id).toBe(id); receipts = [receipt(id, 'succeeded')]; return receipts[0]; }
  return original(command, args);
 });
 const first = await show();
 fireEvent.change(screen.getByLabelText('delivery.directory'), { target: { value: '/new-worktree' } });
 fireEvent.change(screen.getByLabelText('delivery.branch'), { target: { value: 'new-branch' } });
 fireEvent.change(screen.getByLabelText('delivery.baseCommit'), { target: { value: 'a'.repeat(40) } });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.create' })); await act(async () => {});
 first.unmount(); await act(async () => { pending.reject('IPC disconnected'); });
 await show(); expect(screen.getByText('delivery.unknownOperation')).toBeTruthy();
 fireEvent.click(screen.getByRole('button', { name: 'delivery.reconcile' })); await act(async () => {});
 expect(deliveryRequests('alice')).toEqual([]);
 expect(invoke.mock.calls.filter(([command]) => command === 'save_development_workspaces')).toHaveLength(1);
 expect(executionCalls()).toEqual([]);
});
it('isolates unknown identities by task and account while leaving worktree management available', async () => {
 localStorage.setItem('dgos.delivery.requests:alice', JSON.stringify([{ id: 'original-request', kind: 'workspace_create', project_id: 'p', task_id: 't' }]));
 const view = await show();
 expect(screen.getByText('delivery.unknownOperation')).toBeTruthy();
 fireEvent.change(screen.getByLabelText('delivery.directory'), { target: { value: '/import' } });
 expect(screen.getByRole('button', { name: 'delivery.import' })).toHaveProperty('disabled', true);
 view.rerender(<DeliveryPanel {...props} taskId="other" />); await act(async () => {});
 expect(screen.queryByText('delivery.unknownOperation')).toBeNull();
 await act(async () => { useDevelopment.getState().reset(); useDevelopment.setState({ projects: [{ ...project, owner: 'bob' }] }); });
 view.rerender(<DeliveryPanel {...props} />); await act(async () => {});
 expect(screen.queryByText('delivery.unknownOperation')).toBeNull();
});
it('requires trust before inspecting an arbitrary original operation id without replay', async () => {
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
 await show(); fireEvent.change(screen.getByLabelText('delivery.operationId'), { target: { value: 'original-operation' } });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.inspectOperation', hidden: true }));
 fireEvent.click(screen.getByRole('button', { name: 'delivery.reconcile' })); await act(async () => {});
 expect(confirm.mock.calls[0][0].content).toBe('delivery.pushTrust');
 expect(invoke.mock.calls.some(([command]) => command === 'reconcile_development_git')).toBe(false);
 expect(executionCalls()).toEqual([]);
});

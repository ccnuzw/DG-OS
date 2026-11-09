import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter } from 'react-router-dom';
import { Modal } from '@dgos/ui';
import { afterEach, expect, it, vi } from 'vitest';
import { TaskPanel } from '../src/components/development/TaskPanel';
import { ProjectCapabilities } from '../src/components/development/ProjectCapabilities';
import { ProjectPanel } from '../src/components/development/ProjectPanel';
import { DeliveryPanel } from '../src/components/development/DeliveryPanel';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts, guardWorkbenchDrafts } from '../src/stores/developmentDrafts';
import { chooseContext } from '../src/stores/developmentSelection';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); invoke.mockReset(); vi.restoreAllMocks(); });
const project = { id: 'p', owner: 'u', name: 'Project', path: '/project', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: null };
const task = { id: 't', project_id: 'p', owner: 'u', goal: 'Goal', requirements: 'Requirements', status: 'todo' as const, created_at: 1 };

it('retains blank and edited task fields through task/project selection and same-object reload, then confirms discard accurately', async () => {
 invoke.mockResolvedValue(null);
 useDevelopment.setState({ projects: [project], tasks: [task, { ...task, id: 't2' }], selection: { project_id: 'p', task_id: 't', session_id: null } });
 const view = render(<TaskPanel project={project} />);
 fireEvent.change(screen.getByTestId('dev-task-goal'), { target: { value: '' } });
 fireEvent.change(screen.getByTestId('dev-task-requirements'), { target: { value: 'Draft requirements' } });
 await act(async () => { await chooseContext({ project_id: 'p', task_id: 't2', session_id: null }); });
 expect((screen.getByTestId('dev-task-requirements') as HTMLTextAreaElement).value).toBe('Requirements');
 view.rerender(<TaskPanel project={{ ...project, id: 'p2' }} />);
 await act(async () => { await chooseContext({ project_id: 'p', task_id: 't', session_id: null }); useDevelopment.setState({ tasks: [{ ...task, goal: 'Reloaded', requirements: 'Reloaded requirements' }] }); });
 view.rerender(<TaskPanel project={project} />);
 expect((screen.getByTestId('dev-task-goal') as HTMLTextAreaElement).value).toBe('');
 expect((screen.getByTestId('dev-task-requirements') as HTMLTextAreaElement).value).toBe('Draft requirements');
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
 fireEvent.click(screen.getByRole('button', { name: 'dev.discardDraft', hidden: true }));
 await act(async () => {});
 expect(confirm.mock.calls[0][0].content).toBe('dev.discardDraft.task');
 expect((screen.getByTestId('dev-task-requirements') as HTMLTextAreaElement).value).toBe('Draft requirements');
 confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
 fireEvent.click(screen.getByRole('button', { name: 'dev.discardDraft', hidden: true }));
 await act(async () => {});
 expect((screen.getByTestId('dev-task-goal') as HTMLTextAreaElement).value).toBe('Reloaded');
});

it('retains Provider binding drafts across reload, unmount and project changes, then clears them on account reset', async () => {
 invoke.mockImplementation(async (command, args) => command === 'read_development_cli_provider_catalog'
  ? { project_id: args.projectId, engine: args.engine, providers: [{ id: 'provider', name: 'Provider', enabled: true, config_revision: 2, availability: { available: true, reason: null }, models: [{ id: 'model', name: 'Model', model_type: 'chat' }] }] }
  : null);
 const source = () => screen.getByRole('combobox', { name: 'capabilities.source' }) as HTMLSelectElement;
 let view = render(<MemoryRouter><ProjectCapabilities projectId="p" /></MemoryRouter>);
 fireEvent.change(await screen.findByRole('combobox', { name: 'capabilities.source' }), { target: { value: 'provider' } });
 fireEvent.click(screen.getByRole('button', { name: 'capabilities.reload' }));
 await waitFor(() => expect(source().value).toBe('provider'));
 view.unmount();
 await act(async () => { await chooseContext({ project_id: 'p', task_id: 'other', session_id: null }); });
 view = render(<MemoryRouter><ProjectCapabilities projectId="p2" /></MemoryRouter>);
 expect((await screen.findByRole('combobox', { name: 'capabilities.source' }) as HTMLSelectElement).value).toBe('');
 view.rerender(<MemoryRouter><ProjectCapabilities projectId="p" /></MemoryRouter>);
 await waitFor(() => expect(source().value).toBe('provider'));
 vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
 fireEvent.click(screen.getByRole('button', { name: 'capabilities.revoke' }));
 await act(async () => {});
 expect(source().value).toBe('provider');
 expect(invoke.mock.calls.some(([command]) => command === 'revoke_development_cli_provider')).toBe(false);
 await act(async () => { useDevelopment.getState().reset(); });
 await waitFor(() => expect(source().value).toBe(''));
 expect(useDevelopmentDrafts.getState().entries).toEqual({});
});

it('restores commit and per-handoff summary drafts after tab/task changes without transferring them to another object', async () => {
 const handoffs = ['h1', 'h2'].map(id => ({ id, owner: 'u', project_id: 'p', task_id: 't', source_session_id: 's', target_engine: 'claude', permission_epoch: 0, summary: `Prepared ${id}`, fingerprint: id, created_at: 1 }));
 invoke.mockImplementation(async command => {
  if (command === 'read_development_workspaces') return { workspaces: [{ id: 'w', kind: 'original', path: '/project' }], selected_workspace_id: 'w' };
  if (command === 'read_development_git') return { kind: 'git', path: '/project', files: [], conflicts: [], initial_staged_paths: [], staged_diff: '', worktree_diff: '' };
  if (command === 'list_development_handoffs') return handoffs;
  return null;
 });
 const tree = (id: string) => <DeliveryPanel projectId="p" taskId={id} sessions={[]} live={{}} capabilityRevision={0} workspacePath="/project" />;
 let view = render(tree('t'));
 fireEvent.change(await screen.findByLabelText('delivery.commitMessage'), { target: { value: 'Commit draft' } });
 fireEvent.click(screen.getAllByRole('button', { name: 'delivery.useDraft', hidden: true })[0]);
 fireEvent.change(screen.getByLabelText('delivery.summary'), { target: { value: 'Edited h1' } });
 fireEvent.click(screen.getAllByRole('button', { name: 'delivery.useDraft', hidden: true })[1]);
 expect((screen.getByLabelText('delivery.summary') as HTMLTextAreaElement).value).toBe('Prepared h2');
 fireEvent.click(screen.getAllByRole('button', { name: 'delivery.useDraft', hidden: true })[0]);
 expect((screen.getByLabelText('delivery.summary') as HTMLTextAreaElement).value).toBe('Edited h1');
 view.unmount(); view = render(tree('t2'));
 expect((await screen.findByLabelText('delivery.commitMessage') as HTMLInputElement).value).toBe('');
 view.rerender(tree('t'));
 await waitFor(() => expect((screen.getByLabelText('delivery.commitMessage') as HTMLInputElement).value).toBe('Commit draft'));
 expect((screen.getByLabelText('delivery.summary') as HTMLTextAreaElement).value).toBe('Edited h1');
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
 expect(await guardWorkbenchDrafts()).toBe(false);
 expect(confirm.mock.calls[0][0].content).toContain('dev.discardDraft.commit');
 expect(confirm.mock.calls[0][0].content).toContain('dev.discardDraft.handoff');
});

it('retains separate workspace commit drafts and confirms the removed workspace draft before deletion', async () => {
 useDevelopment.setState({ projects: [project] });
 let selected = 'w1';
 const operations: { id: string; kind: string; status: string }[] = [];
 const workspaces = [{ id: 'w1', kind: 'original', path: '/project' }, { id: 'w2', kind: 'managed', path: '/project-worktree' }];
 invoke.mockImplementation(async (command, args) => {
  if (command === 'read_development_workspaces') return { workspaces, selected_workspace_id: selected, operations };
  if (command === 'save_development_workspaces') { selected = args.input.workspace_id; operations.push({ id: args.input.request_id, kind: 'workspace_select', status: 'succeeded' }); return { workspaces, selected_workspace_id: selected, operations }; }
  if (command === 'read_development_git') return { kind: 'git', path: '/project', files: [], conflicts: [], initial_staged_paths: [], staged_diff: '', worktree_diff: '' };
  if (command === 'list_development_handoffs') return [];
  if (command === 'read_development_workspace') return { projects: [project], tasks: [], sessions: [], operations: [], selection: { project_id: 'p', task_id: 't', session_id: null } };
  return null;
 });
 render(<DeliveryPanel projectId="p" taskId="t" sessions={[]} live={{}} capabilityRevision={0} workspacePath="/project" />);
 fireEvent.change(await screen.findByLabelText('delivery.commitMessage'), { target: { value: 'Original commit' } });
 fireEvent.click(screen.getAllByRole('button', { name: 'delivery.select' })[1]);
 await waitFor(() => expect((screen.getByLabelText('delivery.commitMessage') as HTMLInputElement).value).toBe(''));
 fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Managed commit' } });
 fireEvent.click(screen.getAllByRole('button', { name: 'delivery.select' })[0]);
 await waitFor(() => expect((screen.getByLabelText('delivery.commitMessage') as HTMLInputElement).value).toBe('Original commit'));
 fireEvent.change(screen.getByLabelText('delivery.retainBranch'), { target: { value: 'retained' } });
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.remove' }));
 await act(async () => {});
 expect(confirm.mock.calls[0][0].content).toBe('delivery.removeWarning\ndev.discardDraft.commit');
 expect(invoke.mock.calls.filter(([command, args]) => command === 'save_development_workspaces' && args.input.action === 'remove')).toHaveLength(0);
 expect(Object.values(useDevelopmentDrafts.getState().entries).map(item => item.value)).toContain('Managed commit');
});

it('does not discard edits made while a discard dialog is open', async () => {
 invoke.mockResolvedValue(null);
 useDevelopment.setState({ tasks: [task], selection: { project_id: 'p', task_id: 't', session_id: null } });
 render(<TaskPanel project={project} />);
 fireEvent.change(screen.getByTestId('dev-task-goal'), { target: { value: 'First edit' } });
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(() => ({ id: 'confirm', close: () => {} }));
 fireEvent.click(screen.getByRole('button', { name: 'dev.discardDraft', hidden: true }));
 fireEvent.change(screen.getByTestId('dev-task-goal'), { target: { value: 'Later edit' } });
 await act(async () => { await confirm.mock.calls[0][0].onConfirm?.(); });
 expect((screen.getByTestId('dev-task-goal') as HTMLTextAreaElement).value).toBe('Later edit');
});

it('retains failed task saves and isolates a new account using the same project and task ids', async () => {
 invoke.mockRejectedValue('storage_failed');
 useDevelopment.setState({ tasks: [task], selection: { project_id: 'p', task_id: 't', session_id: null } });
 render(<TaskPanel project={project} />);
 fireEvent.change(screen.getByTestId('dev-task-goal'), { target: { value: 'Private draft' } });
 fireEvent.submit(screen.getByTestId('dev-task-form'));
 await screen.findByRole('alert');
 expect((screen.getByTestId('dev-task-goal') as HTMLTextAreaElement).value).toBe('Private draft');
 await act(async () => { useDevelopment.getState().reset(); useDevelopment.setState({ tasks: [task], selection: { project_id: 'p', task_id: 't', session_id: null } }); });
 expect((screen.getByTestId('dev-task-goal') as HTMLTextAreaElement).value).toBe('Goal');
 expect(useDevelopmentDrafts.getState().entries).toEqual({});
});

it('blocks project unlink when discarding its task draft is cancelled', async () => {
 invoke.mockResolvedValue(null);
 useDevelopment.setState({ projects: [project], tasks: [task], selection: { project_id: 'p', task_id: 't', session_id: null } });
 render(<><ProjectPanel /><TaskPanel project={project} /></>);
 fireEvent.change(screen.getByTestId('dev-task-goal'), { target: { value: 'Unlinked draft' } });
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { if (options.title === 'dev.unlink') options.onConfirm?.(); else options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
 await act(async () => { fireEvent.click(screen.getByTestId('dev-project-unlink')); });
 expect(confirm.mock.calls.map(([options]) => options.content)).toContain('dev.discardDraft.task');
 expect(invoke.mock.calls.some(([command]) => command === 'update_development_project')).toBe(false);
 expect((screen.getByTestId('dev-task-goal') as HTMLTextAreaElement).value).toBe('Unlinked draft');
});

import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Modal } from '@dgos/ui';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ChangeDiff } from '../src/components/development/ProjectReview';
import { Files } from '../src/components/development/ProjectFiles';
import { DeliveryPanel } from '../src/components/development/DeliveryPanel';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { useDevelopmentEditor } from '../src/stores/developmentEditor';
const { invoke, confirmDiscard } = vi.hoisted(() => ({ invoke: vi.fn(), confirmDiscard: vi.fn() }));
vi.mock('../src/workspace/closeCoordinator', () => ({ confirmDiscard }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
const project = { id: 'p', owner: 'alice', name: 'Project', path: '/project', directory_identity: 'root', archived: false, created_at: 1, last_opened_at: null };
const overview = { kind: 'git', path: '/project', branch: 'main', head: 'a'.repeat(40), files: [{ path: 'src/app.ts', index: ' ', worktree: 'M' }], conflicts: [], staged_diff: '', worktree_diff: 'changed' };
const review = { ...overview, task_id: 't', review_token: 'task-authority', baseline_version: '', initial_staged_paths: [], file_versions: {}, merge_heads: [], index: [] };
const workspace = { workspaces: [{ id: 'root', path: '/project', kind: 'original' }], selected_workspace_id: null, operations: [], operation: null };
beforeEach(() => {
  localStorage.clear(); useDevelopment.getState().reset(); useDevelopment.setState({ projects: [project] });
  confirmDiscard.mockReset(); confirmDiscard.mockResolvedValue(true);
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_project_git') return overview;
    if (command === 'read_development_git') return review;
    if (command === 'read_development_workspaces') return workspace;
    if (command === 'read_development_workspace') return { projects: [project], tasks: [], sessions: [], operations: [], selection: { project_id: 'p', task_id: null, session_id: null } };
    if (command === 'read_development_project_git_history') return { anchor: overview.head, commits: [], next_cursor: null };
    if (command === 'list_development_files') return { entries: [], truncated: false };
    return null;
  });
});
afterEach(() => { cleanup(); invoke.mockReset(); vi.restoreAllMocks(); useDevelopment.getState().reset(); useDevelopmentDrafts.setState({ entries: {}, handoffs: {} }); localStorage.clear(); });
it('browses project Git and fixed-anchor history without obtaining write authority', async () => {
  const ensureTask = vi.fn();
  render(<DeliveryPanel projectId="p" taskId={null} ensureTask={ensureTask} sessions={[]} live={{}} mode="git" />);
  await screen.findByRole('checkbox', { name: 'src/app.ts' });
  expect(ensureTask).not.toHaveBeenCalled();
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_git')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'delivery.openGitHistory' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('read_development_project_git_history', { input: { project_id: 'p', start_oid: overview.head, cursor: null, limit: 30 } }));
  expect(ensureTask).not.toHaveBeenCalled();
});
it('rereads full task review before an explicit stage and never uses project data as authority', async () => {
  const ensureTask = vi.fn().mockResolvedValue('t');
  invoke.mockImplementation(async (command: string, args: { input: { id: string } }) => {
    if (command === 'read_development_project_git') return overview;
    if (command === 'read_development_git') return review;
    if (command === 'read_development_workspaces') return workspace;
    if (command === 'read_development_workspace') return { projects: [project], tasks: [], sessions: [], operations: [], selection: { project_id: 'p', task_id: null, session_id: null } };
    if (command === 'execute_development_git') return { id: args.input.id, project_id: 'p', task_id: 't', kind: 'git_stage', status: 'succeeded', workspace_id: null, request_fingerprint: '', details: {} };
    return null;
  });
  render(<DeliveryPanel projectId="p" taskId={null} ensureTask={ensureTask} sessions={[]} live={{}} mode="git" />);
  fireEvent.click(await screen.findByRole('checkbox', { name: 'src/app.ts' }));
  fireEvent.click(screen.getByRole('button', { name: 'delivery.stage' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git')).toBe(true));
  const write = invoke.mock.calls.find(([command]) => command === 'execute_development_git')!;
  expect(write[1].input).toMatchObject({ task_id: 't', review_token: 'task-authority', action: { kind: 'stage', paths: ['src/app.ts'] } });
  expect(invoke.mock.calls.findIndex(([command]) => command === 'read_development_git')).toBeLessThan(invoke.mock.calls.findIndex(([command]) => command === 'execute_development_git'));
  expect(ensureTask).toHaveBeenCalledTimes(1);
});
it('never dispatches or retries when explicit task preparation is unconfirmed', async () => {
  const ensureTask = vi.fn().mockResolvedValue(null);
  render(<DeliveryPanel projectId="p" taskId={null} ensureTask={ensureTask} sessions={[]} live={{}} mode="git" />);
  fireEvent.click(await screen.findByRole('checkbox', { name: 'src/app.ts' })); fireEvent.click(screen.getByRole('button', { name: 'delivery.stage' }));
  await act(async () => {});
  expect(ensureTask).toHaveBeenCalledTimes(1);
  expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git')).toBe(false);
});
it('keeps project diff reads taskless', async () => {
  invoke.mockResolvedValue({ path: 'src/app.ts', before: 'before', after: 'after', diff: '-before\n+after', reason: null });
  render(<ChangeDiff projectId="p" taskId={null} path="src/app.ts" source="index" />);
  await screen.findByTestId('dev-file-diff'); await act(async () => {});
  expect(invoke).toHaveBeenCalledWith('read_development_project_diff', { input: { project_id: 'p', path: 'src/app.ts', source: 'index' } });
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_diff')).toBe(false);
});
it('expands directories lazily, retains filter ancestors and creates entries only after trust', async () => {
  invoke.mockImplementation(async (command: string, args: { input: { path: string; query: string; kind: 'file' | 'directory' } }) => {
    if (command === 'list_development_files') {
      if (args.input.query) return { entries: [{ path: 'src/app.ts', name: 'app.ts', kind: 'file', ignored: false, size: 1 }], truncated: false };
      return { entries: args.input.path ? [{ path: 'src/app.ts', name: 'app.ts', kind: 'file', ignored: false, size: 1 }] : [{ path: 'src', name: 'src', kind: 'directory', ignored: false, size: 0 }], truncated: false };
    }
    if (command === 'create_development_entry') return { path: args.input.path, kind: args.input.kind };
    return null;
  });
  const onOpen = vi.fn();
  render(<Files projectId="p" taskId={null} onOpenFile={onOpen} />);
  await screen.findByTestId('dev-file-item-src');
  expect(screen.queryByTestId('dev-file-item-src/app.ts')).toBeNull();
  expect(invoke.mock.calls.filter(([command]) => command === 'list_development_files')).toHaveLength(1);
  fireEvent.click(screen.getByTestId('dev-file-item-src'));
  fireEvent.click(await screen.findByTestId('dev-file-item-src/app.ts'));
  expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ path: 'src/app.ts', mode: 'file' }));
  fireEvent.change(screen.getByTestId('dev-file-search'), { target: { value: 'app' } });
  await screen.findByTestId('dev-file-item-src/app.ts'); expect(screen.getByTestId('dev-file-item-src')).toBeTruthy();
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFolder' }));
  fireEvent.change(screen.getByLabelText('workbenchFiles.entryName'), { target: { value: 'new-folder' } });
  fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'workbenchFiles.create' })); await act(async () => {});
  expect(invoke.mock.calls.some(([command]) => command === 'create_development_entry')).toBe(false);
  confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'workbenchFiles.create' })); await act(async () => {});
  expect(invoke).toHaveBeenCalledWith('create_development_entry', { input: { project_id: 'p', task_id: null, path: 'src/new-folder', kind: 'directory', trust_directory: true } });
});
it('refuses task preparation that resolves to a different workspace', async () => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'read_development_git' ? { ...review, path: '/other-worktree' } : original(command, args));
  render(<DeliveryPanel projectId="p" taskId={null} ensureTask={vi.fn().mockResolvedValue('t')} sessions={[]} live={{}} mode="git" />);
  fireEvent.click(await screen.findByRole('checkbox', { name: 'src/app.ts' })); fireEvent.click(screen.getByRole('button', { name: 'delivery.stage' }));
  await screen.findByRole('alert');
  expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git')).toBe(false);
});

async function createFile(name: string) {
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFile' }));
  fireEvent.change(screen.getByLabelText('workbenchFiles.entryName'), { target: { value: name } });
  fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'workbenchFiles.create' }));
  await act(async () => {});
}
it('retains pending creation across unmount and requires a successful read-only inspection after an unknown result', async () => {
  const pending = Promise.withResolvers<never>();
  let inspectionFails = true;
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === 'create_development_entry') return pending.promise;
    if (command === 'inspect_development_entry') { if (inspectionFails) throw 'file_access_denied'; throw 'file_not_found'; }
    return original(command, args);
  });
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  const onOpen = vi.fn();
  const first = render(<Files projectId="p" taskId={null} onOpenFile={onOpen} />);
  await screen.findByText('review.noFiles'); await createFile('new.txt');
  first.unmount();
  const second = render(<Files projectId="p" taskId={null} onOpenFile={onOpen} />);
  await screen.findByText('review.noFiles');
  expect(screen.getByTestId('dev-review-refresh')).toHaveProperty('disabled', true);
  await act(async () => { pending.reject('file_create_unknown'); });
  fireEvent.click(screen.getByTestId('dev-review-refresh')); await act(async () => {});
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', true);
  inspectionFails = false;
  fireEvent.click(screen.getByTestId('dev-review-refresh')); await act(async () => {});
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', false);
  expect(invoke.mock.calls.filter(([command]) => command === 'create_development_entry')).toHaveLength(1);
  expect(onOpen).not.toHaveBeenCalled();
  second.unmount();
});

it('settles a late create without navigating a changed Files scope or restoring another account draft', async () => {
  const pending = Promise.withResolvers<{ path: string; kind: 'file' }>();
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'create_development_entry' ? pending.promise : original(command, args));
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  const onOpen = vi.fn();
  const { rerender } = render(<Files projectId="p" taskId={null} onOpenFile={onOpen} />);
  await screen.findByText('review.noFiles'); await createFile('new.txt');
  rerender(<Files projectId="p" taskId="other" onOpenFile={onOpen} />);
  await act(async () => { pending.resolve({ path: 'new.txt', kind: 'file' }); });
  expect(onOpen).not.toHaveBeenCalled();
  expect(Object.values(useDevelopmentDrafts.getState().entries).every(entry => !entry.value || typeof entry.value !== 'object' || !('operation' in entry.value) || !entry.value.operation)).toBe(true);
  act(() => { useDevelopment.getState().reset(); });
  expect(useDevelopmentDrafts.getState().entries).toEqual({});
});

const fileEntry = { path: 'app.ts', name: 'app.ts', kind: 'file', ignored: false, size: 1 };
function fileCommands(command: string, args: { input: { path: string } }) {
  if (command === 'list_development_files') return Promise.resolve({ entries: [fileEntry], truncated: false });
  if (command === 'inspect_development_entry') return Promise.resolve({ path: args.input.path, kind: 'file', version: 'original-version' });
  if (command === 'read_development_file') return Promise.resolve({ path: args.input.path, hash: 'hash', text: 'saved', reason: null, size: 5 });
  return Promise.resolve(null);
}
it('preserves a dirty editor on failed rename and only remaps it after successful mutation', async () => {
  invoke.mockImplementation(fileCommands);
  useDevelopment.setState(state => ({ ...state, selection: { project_id: 'p', task_id: 't', session_id: null } }));
  await useDevelopmentEditor.getState().open('p', 't', 'app.ts');
  useDevelopmentEditor.getState().edit('unsaved');
  const onMutation = vi.fn();
  render(<Files projectId="p" taskId="t" onOpenFile={vi.fn()} onMutation={onMutation} />);
  const row = await screen.findByTestId('dev-file-item-app.ts');
  fireEvent.keyDown(row, { key: 'F2' });
  fireEvent.change(await screen.findByLabelText('workbenchFiles.entryName'), { target: { value: 'renamed.ts' } });
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args: unknown) => command === 'mutate_development_entry' ? Promise.reject('file_exists') : original(command, args));
  fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'workbenchFiles.rename' })); await act(async () => {});
  expect(useDevelopmentEditor.getState().draft).toBe('unsaved');
  expect(useDevelopmentEditor.getState().path).toBe('app.ts');
  expect(onMutation).not.toHaveBeenCalled();
  invoke.mockImplementation((command: string, args: unknown) => command === 'mutate_development_entry' ? Promise.resolve({ path: 'renamed.ts', kind: 'file' }) : original(command, args));
  fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'workbenchFiles.rename' })); await act(async () => {});
  expect(onMutation).toHaveBeenCalledWith({ action: 'rename', path: 'app.ts', destination: 'renamed.ts' });
  expect(useDevelopmentEditor.getState().path).toBe('renamed.ts');
  expect(useDevelopmentEditor.getState().file?.path).toBe('renamed.ts');
  expect(useDevelopmentEditor.getState().draft).toBe('unsaved');
  expect(invoke).toHaveBeenCalledWith('mutate_development_entry', { input: { project_id: 'p', task_id: 't', path: 'app.ts', expected_version: 'original-version', action: 'rename', destination: 'renamed.ts', trust_directory: true } });
});

it('captures copy versions and pastes in place under a new name without intercepting input shortcuts', async () => {
  invoke.mockImplementation(fileCommands);
  render(<Files projectId="p" taskId={null} workspacePath="/project" onOpenFile={vi.fn()} />);
  const row = await screen.findByTestId('dev-file-item-app.ts');
  fireEvent.keyDown(screen.getByTestId('dev-file-search'), { key: 'c', ctrlKey: true });
  expect(invoke.mock.calls.some(([command]) => command === 'inspect_development_entry')).toBe(false);
  fireEvent.keyDown(row, { key: 'c', ctrlKey: true }); await act(async () => {});
  fireEvent.keyDown(row, { key: 'v', ctrlKey: true });
  expect(await screen.findByLabelText('workbenchFiles.entryName')).toHaveProperty('value', 'app copy.ts');
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation((command: string, args: unknown) => command === 'mutate_development_entry' ? Promise.resolve({ path: 'app copy.ts', kind: 'file' }) : original(command, args));
  fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'workbenchFiles.paste' })); await act(async () => {});
  expect(invoke).toHaveBeenCalledWith('mutate_development_entry', { input: { project_id: 'p', task_id: null, path: 'app.ts', expected_version: 'original-version', action: 'copy', destination: 'app copy.ts', trust_directory: true } });
});

it('drops a copy inspection completed after a scope switch', async () => {
  const inspected = Promise.withResolvers<unknown>();
  invoke.mockImplementation((command: string, args: { input: { path: string } }) => command === 'inspect_development_entry' ? inspected.promise : fileCommands(command, args));
  const { rerender } = render(<Files projectId="p" taskId={null} workspacePath="/project" onOpenFile={vi.fn()} />);
  fireEvent.keyDown(await screen.findByTestId('dev-file-item-app.ts'), { key: 'c', ctrlKey: true });
  rerender(<Files projectId="p" taskId="other" workspacePath="/other" onOpenFile={vi.fn()} />);
  await act(async () => { inspected.resolve({ path: 'app.ts', kind: 'file', version: 'old' }); });
  fireEvent.keyDown(await screen.findByTestId('dev-file-item-app.ts'), { key: 'v', ctrlKey: true });
  expect(screen.queryByRole('dialog')).toBeNull();
});

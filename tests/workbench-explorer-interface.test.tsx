import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Modal } from '@dgos/ui';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Files, ChangeDiff } from '../src/components/development/ProjectReview';
import { DeliveryPanel } from '../src/components/development/DeliveryPanel';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
const project = { id: 'p', owner: 'alice', name: 'Project', path: '/project', directory_identity: 'root', archived: false, created_at: 1, last_opened_at: null };
const overview = { kind: 'git', path: '/project', branch: 'main', head: 'a'.repeat(40), files: [{ path: 'src/app.ts', index: ' ', worktree: 'M' }], conflicts: [], staged_diff: '', worktree_diff: 'changed' };
const review = { ...overview, task_id: 't', review_token: 'task-authority', baseline_version: '', initial_staged_paths: [], file_versions: {}, merge_heads: [], index: [] };
const workspace = { workspaces: [{ id: 'root', path: '/project', kind: 'original' }], selected_workspace_id: null, operations: [], operation: null };
beforeEach(() => {
  localStorage.clear(); useDevelopment.getState().reset(); useDevelopment.setState({ projects: [project] });
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
  fireEvent.change(screen.getByLabelText('workbenchFiles.newPath'), { target: { value: 'new-folder' } });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFolder' })); await act(async () => {});
  expect(invoke.mock.calls.some(([command]) => command === 'create_development_entry')).toBe(false);
  confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFolder' })); await act(async () => {});
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

it('does not replay an unknown entry creation and retains the entered name for inspection', async () => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => { if (command === 'create_development_entry') throw 'file_create_unknown'; return original(command, args); });
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  render(<Files projectId="p" taskId={null} onOpenFile={vi.fn()} />); await screen.findByText('review.noFiles');
  fireEvent.change(screen.getByLabelText('workbenchFiles.newPath'), { target: { value: 'new.txt' } });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFile' })); await screen.findByRole('alert');
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFile' }));
  expect(invoke.mock.calls.filter(([command]) => command === 'create_development_entry')).toHaveLength(1);
  fireEvent.click(screen.getByTestId('dev-review-refresh')); await act(async () => {});
  expect(invoke.mock.calls.filter(([command]) => command === 'create_development_entry')).toHaveLength(1);
  expect(screen.getByLabelText('workbenchFiles.newPath')).toHaveProperty('value', 'new.txt');
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', false);
});

it('retains a pending create across switches and inspects a late unknown result without replay', async () => {
  const { promise: pendingCreate, reject: rejectCreate } = Promise.withResolvers<never>();
  let inspection: 'truncated' | 'failed' | 'complete' = 'truncated';
  invoke.mockImplementation(async (command: string, args: { input: { path: string; query: string; include_ignored: boolean } }) => {
    if (command === 'create_development_entry') return pendingCreate;
    if (command === 'list_development_files') {
      if (args.input.path === 'src' && args.input.include_ignored) {
        if (inspection === 'failed') throw 'file_access_denied';
        return { entries: [{ path: 'src/new.txt', name: 'new.txt', kind: 'file', ignored: true, size: 0 }], truncated: inspection === 'truncated' };
      }
      return { entries: args.input.path ? [] : [{ path: 'src', name: 'src', kind: 'directory', ignored: false, size: 0 }], truncated: false };
    }
    return null;
  });
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  const onOpen = vi.fn();
  const { rerender } = render(<Files key="original" projectId="p" taskId={null} workspacePath="/project" onOpenFile={onOpen} />);
  fireEvent.click(await screen.findByTestId('dev-file-item-src'));
  fireEvent.change(screen.getByLabelText('workbenchFiles.newPath'), { target: { value: 'new.txt' } });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFile' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('create_development_entry', { input: { project_id: 'p', task_id: null, path: 'src/new.txt', kind: 'file', trust_directory: true } }));
  rerender(<Files key="other" projectId="p" taskId="other" workspacePath="/other" onOpenFile={onOpen} />);
  await screen.findByTestId('dev-file-item-src');
  expect(screen.getByLabelText('workbenchFiles.newPath')).toHaveProperty('value', '');
  rerender(<Files key="original" projectId="p" taskId={null} workspacePath="/project" onOpenFile={onOpen} />);
  await screen.findByTestId('dev-file-item-src');
  expect(screen.getByLabelText('workbenchFiles.newPath')).toHaveProperty('value', 'new.txt');
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', true);
  expect(screen.getByTestId('dev-review-refresh')).toHaveProperty('disabled', true);
  rerender(<Files key="other" projectId="p" taskId="other" workspacePath="/other" onOpenFile={onOpen} />);
  await act(async () => { rejectCreate('file_create_unknown'); });
  expect(screen.queryByText('dev.error.file_create_unknown')).toBeNull();
  rerender(<Files key="original" projectId="p" taskId={null} workspacePath="/project" onOpenFile={onOpen} />);
  await screen.findByText('src/new.txt');
  await screen.findByTestId('dev-file-item-src');
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', true);
  fireEvent.change(screen.getByTestId('dev-file-search'), { target: { value: 'filtered' } });
  await act(async () => {});
  fireEvent.click(screen.getByTestId('dev-review-refresh')); await act(async () => {});
  expect(invoke).toHaveBeenCalledWith('list_development_files', { input: { project_id: 'p', task_id: null, path: 'src', query: '', include_ignored: true } });
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', true);
  inspection = 'failed';
  fireEvent.click(screen.getByTestId('dev-review-refresh')); await act(async () => {});
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', true);
  inspection = 'complete';
  fireEvent.click(screen.getByTestId('dev-review-refresh')); await act(async () => {});
  expect(screen.queryByText('src/new.txt')).toBeNull();
  expect(screen.getByLabelText('workbenchFiles.newPath')).toHaveProperty('value', '');
  fireEvent.change(screen.getByLabelText('workbenchFiles.newPath'), { target: { value: 'next.txt' } });
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', false);
  expect(invoke.mock.calls.filter(([command]) => command === 'create_development_entry')).toHaveLength(1);
  expect(onOpen).not.toHaveBeenCalled();
});

it('settles a late successful create without navigating a changed Files scope', async () => {
  const { promise: pendingCreate, resolve: resolveCreate } = Promise.withResolvers<{ path: string; kind: 'file' }>();
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'create_development_entry' ? pendingCreate : original(command, args));
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  const onOpen = vi.fn();
  const { rerender } = render(<Files projectId="p" taskId={null} onOpenFile={onOpen} />);
  await screen.findByText('review.noFiles');
  fireEvent.change(screen.getByLabelText('workbenchFiles.newPath'), { target: { value: 'new.txt' } });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFile' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'create_development_entry')).toBe(true));
  rerender(<Files projectId="p" taskId="other" onOpenFile={onOpen} />);
  await act(async () => { resolveCreate({ path: 'new.txt', kind: 'file' }); });
  expect(onOpen).not.toHaveBeenCalled();
  rerender(<Files projectId="p" taskId={null} onOpenFile={onOpen} />);
  await screen.findByText('review.noFiles');
  expect(screen.getByLabelText('workbenchFiles.newPath')).toHaveProperty('value', '');
  fireEvent.change(screen.getByLabelText('workbenchFiles.newPath'), { target: { value: 'next.txt' } });
  expect(screen.getByRole('button', { name: 'workbenchFiles.newFile' })).toHaveProperty('disabled', false);
  expect(invoke.mock.calls.filter(([command]) => command === 'create_development_entry')).toHaveLength(1);
});

it('does not restore a late unknown create into a new account epoch', async () => {
  const { promise: pendingCreate, reject: rejectCreate } = Promise.withResolvers<never>();
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'create_development_entry' ? pendingCreate : original(command, args));
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  const onOpen = vi.fn();
  const { unmount } = render(<Files projectId="p" taskId={null} onOpenFile={onOpen} />);
  await screen.findByText('review.noFiles');
  fireEvent.change(screen.getByLabelText('workbenchFiles.newPath'), { target: { value: 'new.txt' } });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.newFile' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'create_development_entry')).toBe(true));
  unmount();
  act(() => { useDevelopment.getState().reset(); useDevelopment.setState({ projects: [project] }); });
  await act(async () => { rejectCreate('file_create_unknown'); });
  render(<Files projectId="p" taskId={null} onOpenFile={onOpen} />);
  await screen.findByText('review.noFiles');
  expect(useDevelopmentDrafts.getState().entries).toEqual({});
  expect(screen.getByLabelText('workbenchFiles.newPath')).toHaveProperty('value', '');
  expect(onOpen).not.toHaveBeenCalled();
});

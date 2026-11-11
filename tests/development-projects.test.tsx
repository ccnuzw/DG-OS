import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Modal, ModalHost } from '@dgos/ui';
import { removeDevelopmentProject } from '../src/components/development/removeProject';
import { documentId, useDevelopmentEditor } from '../src/stores/developmentEditor';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { ProjectPanel } from '../src/components/development/ProjectPanel';
import { useDevelopment } from '../src/stores/developmentStore';
const { picker, invoke } = vi.hoisted(() => ({ picker: vi.fn(), invoke: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: picker }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/path', () => ({ basename: async (path: string) => path.split('/').pop(), join: async (...parts: string[]) => parts.join('/') }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
afterEach(() => { cleanup(); Modal.closeAll(); useDevelopment.getState().reset(); vi.restoreAllMocks(); picker.mockReset(); invoke.mockReset(); });
async function action(name: string) {
  fireEvent.keyDown(screen.getByRole('button', { name: 'dev.add' }), { key: 'Enter' });
  fireEvent.click(await screen.findByRole('menuitem', { name }));
}
it('keeps failed creation editable, blocks duplicate submission and closes only after success', async () => {
  let finish!: (value: { ok: false; code: string }) => void;
  const create = vi.spyOn(useDevelopment.getState(), 'createProject').mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  picker.mockResolvedValue('/projects');
  render(<ProjectPanel />);
  await action('dev.createFolder');
  const submit = screen.getByRole('button', { name: 'dev.create', exact: true });
  expect((submit as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(screen.getByLabelText('dev.name'), { target: { value: '../escape' } });
  fireEvent.click(screen.getByRole('button', { name: 'dev.chooseParent' }));
  await waitFor(() => expect((screen.getByLabelText('dev.parentFolder') as HTMLInputElement).value).toBe('/projects'));
  expect((submit as HTMLButtonElement).disabled).toBe(true);
  fireEvent.change(screen.getByLabelText('dev.name'), { target: { value: 'Demo' } });
  fireEvent.click(submit); fireEvent.click(submit);
  await waitFor(() => expect(create).toHaveBeenCalledTimes(1));
  fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
  expect(screen.getByRole('dialog')).toBeTruthy();
  await act(async () => finish({ ok: false, code: 'unknown' }));
  expect((screen.getByLabelText('dev.name') as HTMLInputElement).value).toBe('Demo');
  expect(screen.getByRole('alert')).toBeTruthy();
  create.mockResolvedValue({ ok: true, value: { id: 'p', owner: 'u', name: 'Demo', path: '/projects/Demo', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: null } });
  invoke.mockResolvedValue(null);
  fireEvent.click(submit);
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(useDevelopment.getState().selection.project_id).toBe('p');
});
it('derives the clone directory from the repository and preserves a custom name', async () => {
  const create = vi.spyOn(useDevelopment.getState(), 'createProject').mockResolvedValue({ ok: false, code: 'unknown' });
  picker.mockResolvedValue('/projects');
  render(<ProjectPanel />);
  await action('dev.cloneRepository');
  fireEvent.change(screen.getByLabelText('dev.source'), { target: { value: 'git@github.com:team/example.git' } });
  fireEvent.click(screen.getByRole('button', { name: 'dev.chooseParent' }));
  await screen.findByText('/projects/example');
  fireEvent.click(screen.getByRole('button', { name: 'dev.cloneRepositoryTitle' }));
  await waitFor(() => expect(screen.getByRole('alert')).toBeTruthy());
  expect(create).toHaveBeenLastCalledWith({ kind: 'clone', name: 'example', path: '/projects/example', source: 'git@github.com:team/example.git' });
  fireEvent.change(screen.getByLabelText('dev.name'), { target: { value: 'Custom' } });
  fireEvent.click(screen.getByRole('button', { name: 'dev.cloneRepositoryTitle' }));
  await waitFor(() => expect(create).toHaveBeenLastCalledWith(expect.objectContaining({ name: 'Custom', path: '/projects/Custom' })));
});
it('cancels folder selection without mutation and reports picker errors without opening a creation dialog', async () => {
  const create = vi.spyOn(useDevelopment.getState(), 'createProject');
  picker.mockResolvedValueOnce(null).mockRejectedValueOnce(new Error('picker unavailable'));
  render(<ProjectPanel />);
  await action('dev.openFolder');
  await waitFor(() => expect((screen.getByRole('button', { name: 'dev.add' }) as HTMLButtonElement).disabled).toBe(false));
  expect(create).not.toHaveBeenCalled();
  await action('dev.openFolder');
  expect(await screen.findByRole('alert')).toHaveProperty('textContent', 'dev.folderPickerFailed');
  expect(screen.queryByRole('dialog')).toBeNull();
});

it('does not import an old account folder when its native picker returns late', async () => {
 const pending = Promise.withResolvers<string>(); picker.mockReturnValue(pending.promise);
 const create = vi.spyOn(useDevelopment.getState(), 'createProject').mockResolvedValue({ ok: false, code: 'unknown' });
 render(<ProjectPanel />); await action('dev.openFolder');
 act(() => useDevelopment.getState().reset());
 await act(async () => pending.resolve('/old-account/project'));
 expect(create).not.toHaveBeenCalled();
 expect(useDevelopment.getState().selection.project_id).toBeNull();
});

it('keeps add-only navigation free of project lists, inspection and management forms', () => {
  useDevelopment.setState({ projects: [{ id: 'p', owner: 'u', name: 'Demo', path: '/projects/Demo', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: null }], selection: { project_id: 'p', task_id: null, session_id: null } });
  const inspect = vi.spyOn(useDevelopment.getState(), 'inspect');
  render(<ProjectPanel mode="add-only" />);
  expect(screen.getByRole('button', { name: 'dev.add' })).toBeTruthy();
  expect(screen.queryByTestId('dev-project-item-p')).toBeNull();
  expect(screen.queryByTestId('dev-project-detail')).toBeNull();
  expect(screen.queryByLabelText('workbenchNavigation.searchProjects')).toBeNull();
  expect(inspect).not.toHaveBeenCalled();
});

it('manages the explicit project without selecting it or rendering a second project list', async () => {
  const project = { id: 'p', owner: 'u', name: 'Demo', path: '/projects/Demo', directory_identity: 'd', archived: false, created_at: 1, last_opened_at: null };
  useDevelopment.setState({ projects: [project, { ...project, id: 'other', name: 'Other' }], selection: { project_id: 'other', task_id: null, session_id: null } });
  const inspect = vi.spyOn(useDevelopment.getState(), 'inspect').mockResolvedValue({ ok: true, value: { path_state: 'available', kind: 'directory', repository_root: null, branch: null, changes: [], truncated: false, error: null } });
  render(<ProjectPanel mode="manage" projectId="p" />);
  await waitFor(() => expect(inspect).toHaveBeenCalledWith('p'));
  expect(screen.getByTestId('dev-project-detail').querySelector('details')?.open).toBe(true);
  expect(screen.getByLabelText('dev.renameField')).toHaveProperty('value', 'Demo');
  expect(screen.queryByTestId('dev-project-item-p')).toBeNull();
  expect(useDevelopment.getState().selection.project_id).toBe('other');
});

async function openRemovalDrafts() {
  invoke.mockImplementation((command, args) => command === 'read_development_file' ? Promise.resolve({ path: args.input.path, text: 'base', hash: 'hash', size: 4, reason: null }) : Promise.resolve(null));
  await useDevelopmentEditor.getState().open('p', 't', 'project.ts');
  useDevelopmentEditor.getState().edit('project draft');
  await useDevelopmentEditor.getState().open('other', 'other-task', 'unrelated.ts');
  useDevelopmentEditor.getState().edit('unrelated draft');
  return { project: documentId('p', 't', 'project.ts'), other: documentId('other', 'other-task', 'unrelated.ts') };
}

it.each([true, false])('scopes file consent to the removed project and discards only after host success=%s', async succeeded => {
  const ids = await openRemovalDrafts();
  const update = vi.spyOn(useDevelopment.getState(), 'updateProject').mockResolvedValue(succeeded ? { ok: true, value: null } : { ok: false, code: 'cleanup_pending' });
  render(<ModalHost />);
  const removing = removeDevelopmentProject('p');
  fireEvent.click(await screen.findByRole('button', { name: 'dev.unlinkAndStop' }));
  await screen.findByRole('dialog', { name: 'workbenchFiles.unsavedTitle' });
  expect(screen.getByText('project.ts')).toBeTruthy();
  expect(screen.queryByText('unrelated.ts')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.discardAndContinue' }));
  expect((await removing).ok).toBe(succeeded);
  expect(update).toHaveBeenCalledTimes(1);
  expect(useDevelopmentEditor.getState().documents[ids.project].draft).toBe(succeeded ? 'base' : 'project draft');
  expect(useDevelopmentEditor.getState().documents[ids.other].draft).toBe('unrelated draft');
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_file')).toBe(false);
});

it('retains all file drafts when project form consent is cancelled before unlink', async () => {
  const ids = await openRemovalDrafts();
  const key = developmentDraftKey('task', 'p', 't');
  useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'task', value: { goal: 'unsaved goal' }, baseline: {} } } });
  const update = vi.spyOn(useDevelopment.getState(), 'updateProject');
  const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => {
    if (options.title === 'dev.unlink') options.onConfirm?.(); else options.onCancel?.();
    return { id: 'removal-consent', close: () => {} };
  });
  expect(await removeDevelopmentProject('p')).toEqual({ ok: false, code: 'cancelled' });
  expect(update).not.toHaveBeenCalled();
  expect(confirm.mock.calls.some(([options]) => options.title === 'workbenchFiles.unsavedTitle')).toBe(false);
  expect(useDevelopmentEditor.getState().documents[ids.project].draft).toBe('project draft');
  expect(useDevelopmentEditor.getState().documents[ids.other].draft).toBe('unrelated draft');
  expect(useDevelopmentDrafts.getState().entries[key].value).toEqual({ goal: 'unsaved goal' });
});

it('does not discard newer edits made while project unlink is awaiting its receipt', async () => {
  const ids = await openRemovalDrafts();
  const pending = Promise.withResolvers<{ ok: true; value: null }>();
  const update = vi.spyOn(useDevelopment.getState(), 'updateProject').mockReturnValue(pending.promise);
  render(<ModalHost />);
  const removing = removeDevelopmentProject('p');
  fireEvent.click(await screen.findByRole('button', { name: 'dev.unlinkAndStop' }));
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.discardAndContinue' }));
  await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
  useDevelopmentEditor.getState().edit('late project draft', ids.project);
  pending.resolve({ ok: true, value: null });
  expect((await removing).ok).toBe(true);
  expect(useDevelopmentEditor.getState().documents[ids.project].draft).toBe('late project draft');
  expect(useDevelopmentEditor.getState().documents[ids.other].draft).toBe('unrelated draft');
});

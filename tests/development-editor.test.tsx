import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Modal, ModalHost } from '@dgos/ui';
import { Files } from '../src/components/development/ProjectFiles';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentEditor, documentId, openDevelopmentDocument, closeDevelopmentDocuments, guardDevelopmentDraft, guardDevelopmentDocuments, cancelDevelopmentDocumentEdit, discardDevelopmentDraft, remapDevelopmentDocuments, invalidateDevelopmentDocuments } from '../src/stores/developmentEditor';
import { chooseContext } from '../src/stores/developmentSelection';
import { useDevelopmentRuntime } from '../src/stores/developmentRuntime';
import { FileEditor } from '../src/components/development/FileEditor';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
const { invoke, confirmDiscard } = vi.hoisted(() => ({ invoke: vi.fn(), confirmDiscard: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/workspace/closeCoordinator', () => ({ confirmDiscard }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
afterEach(() => { cleanup(); Modal.closeAll(); useDevelopment.getState().reset(); invoke.mockReset(); confirmDiscard.mockReset(); });
function installFiles() {
  invoke.mockImplementation((command: string, args: { input?: { path: string } }) => {
    if (command === 'list_development_files') return Promise.resolve({ entries: ['a.ts', 'b.ts'].map(path => ({ path, name: path, kind: 'file', size: 4, ignored: false })), truncated: false });
    if (command === 'read_development_file') return Promise.resolve({ path: args.input?.path, text: 'base', hash: 'initial-hash', size: 4, reason: null });
    return Promise.resolve(null);
  });
}
it('retains independent file drafts through filtering and project changes without asking to discard', async () => {
  installFiles(); confirmDiscard.mockResolvedValue(false);
  render(<Files projectId="p" taskId="t" />);
  fireEvent.click(await screen.findByTestId('dev-file-item-a.ts'));
  await screen.findByTestId('dev-file-editor');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.editFile' }));
  fireEvent.change(await screen.findByTestId('dev-file-editor'), { target: { value: 'draft a' } });
  fireEvent.click(screen.getByTestId('dev-file-item-b.ts'));
  await waitFor(() => expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('base'));
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.editFile' }));
  fireEvent.change(screen.getByTestId('dev-file-editor'), { target: { value: 'draft b' } });
  expect(confirmDiscard).not.toHaveBeenCalled();
  fireEvent.click(screen.getByTestId('dev-file-item-a.ts'));
  await waitFor(() => expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('draft a'));
  fireEvent.change(screen.getByTestId('dev-file-search'), { target: { value: 'b' } });
  await act(async () => { expect((await chooseContext({ project_id: 'other', task_id: null, session_id: null })).ok).toBe(true); });
  expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('draft a');
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'b.ts')].draft).toBe('draft b');
  expect((screen.getByTestId('dev-file-search') as HTMLInputElement).value).toBe('b');
  expect(invoke.mock.calls.some(([command]) => command === 'select_development_context')).toBe(true);
  expect(confirmDiscard).not.toHaveBeenCalled();
});
it('retains a conflict draft, compares reread disk and requires explicit discard to reload', async () => {
  installFiles();
  render(<><Files projectId="p" taskId="t" /><ModalHost /></>);
  fireEvent.click(await screen.findByTestId('dev-file-item-a.ts'));
  await screen.findByTestId('dev-file-editor');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.editFile' }));
  fireEvent.change(await screen.findByTestId('dev-file-editor'), { target: { value: 'my draft' } });
  invoke.mockImplementation((command: string) => command === 'save_development_file' ? Promise.reject('file_changed') : Promise.resolve({ path: 'a.ts', text: 'external edit', hash: 'disk-hash', size: 13, reason: null }));
  fireEvent.click(screen.getByTestId('dev-file-save'));
  await screen.findByText('external edit');
  expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('my draft');
  expect(invoke).toHaveBeenCalledWith('save_development_file', { input: { project_id: 'p', task_id: 't', path: 'a.ts', expected_hash: 'initial-hash', text: 'my draft' } });
  fireEvent.click(screen.getByTestId('dev-file-reload'));
  expect(await screen.findByRole('dialog', { name: 'workbenchFiles.unsavedTitle' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'workbenchFiles.saveAndContinue' })).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.returnToEdit' }));
  expect(useDevelopmentEditor.getState().draft).toBe('my draft');
  fireEvent.click(screen.getByTestId('dev-file-reload'));
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.discardAndContinue' }));
  await waitFor(() => expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('external edit'));
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_file')).toHaveLength(1);
});
it('drops a late save response after account reset and clears the draft', async () => {
  installFiles(); await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('private draft');
  const pending = Promise.withResolvers<unknown>(); invoke.mockReturnValue(pending.promise);
  const saving = useDevelopmentEditor.getState().save(); useDevelopment.getState().reset();
  pending.resolve({ path: 'a.ts', text: 'private draft', hash: 'new', size: 13, reason: null }); await saving;
  expect(useDevelopmentEditor.getState().file).toBeNull(); expect(useDevelopmentEditor.getState().draft).toBe('');
});
it('does not apply old task process history after another task or account takes over', async () => {
  const old = Promise.withResolvers<unknown>();
  invoke.mockImplementation((_command: string, args: { taskId: string }) => args.taskId === 'old' ? old.promise : Promise.resolve([]));
  const reading = useDevelopmentRuntime.getState().refresh('old');
  await useDevelopmentRuntime.getState().refresh('new');
  old.resolve([{ id: 'old-process', task_id: 'old', output: 'PRIVATE_OLD_OUTPUT' }]); await reading;
  expect(useDevelopmentRuntime.getState().taskId).toBe('new'); expect(useDevelopmentRuntime.getState().processes).toEqual([]);
  const account = Promise.withResolvers<unknown>(); invoke.mockReturnValue(account.promise);
  const accountRead = useDevelopmentRuntime.getState().refresh('new'); useDevelopment.getState().reset();
  account.resolve([{ id: 'private-process' }]); await accountRead;
  expect(useDevelopmentRuntime.getState().processes).toEqual([]); expect(useDevelopmentRuntime.getState().taskId).toBeNull();
});

it('positions a validated diagnostic line once without resetting later edits', async () => {
  invoke.mockResolvedValue({ path: 'a.ts', text: 'first\nsecond\nthird', hash: 'hash', size: 18, reason: null });
  render(<FileEditor projectId="p" taskId="t" path="a.ts" line={2} />);
  const editor = await screen.findByTestId('dev-file-editor') as HTMLTextAreaElement;
  await waitFor(() => expect(editor.selectionStart).toBe(6));
  expect(editor.selectionEnd).toBe(12);
  editor.setSelectionRange(0, 0);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.editFile' }));
  fireEvent.change(editor, { target: { value: 'changed\nsecond\nthird' } });
  expect(editor.selectionStart).not.toBe(8);
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_file')).toBe(false);
});

it('saves the current draft once after composition ends and ignores repeating save keys', async () => {
  installFiles();
  render(<FileEditor projectId="p" taskId="t" path="a.ts" />);
  const editor = await screen.findByTestId('dev-file-editor');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.editFile' }));
  fireEvent.change(editor, { target: { value: '中文 draft' } });
  fireEvent.keyDown(editor, { key: 's', ctrlKey: true, isComposing: true });
  fireEvent.keyDown(editor, { key: 's', metaKey: true, keyCode: 229 });
  fireEvent.keyDown(editor, { key: 's', ctrlKey: true, repeat: true });
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_file')).toHaveLength(0);
  invoke.mockImplementation((command: string) => command === 'save_development_file' ? Promise.resolve({ path: 'a.ts', text: '中文 draft', hash: 'saved', size: 12, reason: null }) : Promise.resolve(null));
  fireEvent.keyDown(editor, { key: 'S', ctrlKey: true });
  await waitFor(() => expect(useDevelopmentEditor.getState().saving).toBe(false));
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_file')).toEqual([['save_development_file', { input: { project_id: 'p', task_id: 't', path: 'a.ts', expected_hash: 'initial-hash', text: '中文 draft' } }]]);
});

it('references selected saved lines and saves an unsaved selection before adding it', async () => {
  invoke.mockResolvedValue({ path: 'a.ts', text: 'first\nsecond\nthird', hash: 'disk-version', size: 18, reason: null });
  const reference = vi.fn();
  render(<FileEditor projectId="p" taskId="t" path="a.ts" onReference={reference} />);
  const editor = await screen.findByTestId('dev-file-editor') as HTMLTextAreaElement;
  editor.setSelectionRange(6, 13); fireEvent.select(editor);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.addSelection' }));
  expect(reference).toHaveBeenCalledWith({ project_id: 'p', task_id: 't', path: 'a.ts', kind: 'selection', expected_hash: 'disk-version', start_line: 2, end_line: 2 });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.editFile' }));
  fireEvent.change(editor, { target: { value: 'unsaved\nsecond\nthird' } });
  editor.setSelectionRange(0, 7); fireEvent.select(editor);
  invoke.mockResolvedValueOnce({ path: 'a.ts', text: 'unsaved\nsecond\nthird', hash: 'saved-selection', size: 20, reason: null });
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.saveAndAdd' }));
  await waitFor(() => expect(reference).toHaveBeenLastCalledWith({ project_id: 'p', task_id: 't', path: 'a.ts', kind: 'selection', expected_hash: 'saved-selection', start_line: 1, end_line: 1 }));
  expect(invoke).toHaveBeenCalledWith('save_development_file', { input: { project_id: 'p', task_id: 't', path: 'a.ts', expected_hash: 'disk-version', text: 'unsaved\nsecond\nthird' } });
});

it('reports an expired line and never exposes the previous project file during a read', async () => {
  invoke.mockResolvedValue({ path: 'a.ts', text: 'first\nsecond', hash: 'disk-version', size: 12, reason: null });
  const view = render(<FileEditor projectId="p" taskId="t" path="a.ts" line={99} />);
  await screen.findByText('workbenchFiles.invalidLine');
  const next = Promise.withResolvers<unknown>(); invoke.mockReturnValue(next.promise);
  view.rerender(<FileEditor projectId="other" taskId="other-task" path="a.ts" />);
  expect(screen.queryByTestId('dev-file-editor')).toBeNull();
  await act(async () => next.resolve({ path: 'a.ts', text: 'different project', hash: 'other-version', size: 17, reason: null }));
  expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('different project');
});

it('saves two dirty files independently even when another tab is active', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  const a = documentId('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('draft a', a);
  await useDevelopmentEditor.getState().open('p', 't', 'b.ts', '/repo');
  const b = documentId('p', 't', 'b.ts', '/repo');
  useDevelopmentEditor.getState().edit('draft b', b);
  invoke.mockImplementation((command: string, args: { input: { path: string; text: string } }) => command === 'save_development_file' ? Promise.resolve({ path: args.input.path, text: args.input.text, hash: `saved-${args.input.path}`, size: 7, reason: null }) : Promise.resolve(null));
  await useDevelopmentEditor.getState().save(a);
  expect(useDevelopmentEditor.getState().documents[a].file?.text).toBe('draft a');
  expect(useDevelopmentEditor.getState().documents[b].draft).toBe('draft b');
  expect(useDevelopmentEditor.getState().documents[b].file?.text).toBe('base');
  await useDevelopmentEditor.getState().save(b);
  expect(useDevelopmentEditor.getState().documents[b].file?.text).toBe('draft b');
  expect(useDevelopmentEditor.getState().draft).toBe('draft b');
});

it('keeps all tabs intact when batch dirty-close is cancelled', async () => {
  installFiles();
  for (const path of ['a.ts', 'b.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
    useDevelopmentEditor.getState().edit(`draft ${path}`);
  }
  const ids = useDevelopmentEditor.getState().tabs.map(tab => tab.id);
  render(<ModalHost />);
  const cancelled = closeDevelopmentDocuments(ids);
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.returnToEdit' }));
  expect(await cancelled).toBe(false);
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.id)).toEqual(ids);
  const discarded = closeDevelopmentDocuments(ids);
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.discardAndContinue' }));
  expect(await discarded).toBe(true);
  expect(useDevelopmentEditor.getState().tabs).toEqual([]);
  expect(useDevelopmentEditor.getState().documents).toEqual({});
});

it('guards inactive drafts and remaps every document under renamed ancestors', async () => {
  installFiles();
  for (const path of ['folder/a.ts', 'folder/b.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
    useDevelopmentEditor.getState().edit(`draft ${path}`);
  }
  render(<ModalHost />);
  const guarded = guardDevelopmentDraft();
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.returnToEdit' }));
  expect(await guarded).toBe(false);
  remapDevelopmentDocuments('p', 't', 'folder', 'renamed', '/repo');
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.path)).toEqual(['renamed/a.ts', 'renamed/b.ts']);
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'renamed/a.ts', '/repo')].draft).toBe('draft folder/a.ts');
  invalidateDevelopmentDocuments('p', 't', ['renamed'], '/repo');
  expect(useDevelopmentEditor.getState().tabs).toEqual([]);
  expect(useDevelopmentEditor.getState().documents).toEqual({});
});

it('separates equal paths across workspace and project identities and clears all on account reset', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/one');
  useDevelopmentEditor.getState().edit('private draft');
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/two');
  expect(useDevelopmentEditor.getState().draft).toBe('base');
  await useDevelopmentEditor.getState().open('other', 't', 'a.ts', '/one');
  expect(useDevelopmentEditor.getState().draft).toBe('base');
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/one');
  expect(useDevelopmentEditor.getState().draft).toBe('private draft');
  useDevelopment.getState().reset();
  expect(useDevelopmentEditor.getState().documents).toEqual({});
  expect(useDevelopmentEditor.getState().tabs).toEqual([]);
});

it('settles an inactive save without replacing newer typing or another file draft', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  const a = documentId('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('saved snapshot', a);
  const pending = Promise.withResolvers<unknown>();
  invoke.mockImplementation((command: string, args: { input?: { path: string } }) => command === 'save_development_file' ? pending.promise : Promise.resolve({ path: args.input?.path, text: 'base', hash: 'initial-hash', size: 4, reason: null }));
  const saving = useDevelopmentEditor.getState().save(a);
  useDevelopmentEditor.getState().edit('typed during save', a);
  await useDevelopmentEditor.getState().open('p', 't', 'b.ts');
  useDevelopmentEditor.getState().edit('draft b');
  pending.resolve({ path: 'a.ts', text: 'saved snapshot', hash: 'saved-hash', size: 14, reason: null });
  await saving;
  expect(useDevelopmentEditor.getState().documents[a].draft).toBe('typed during save');
  expect(useDevelopmentEditor.getState().documents[a].file?.hash).toBe('saved-hash');
  expect(useDevelopmentEditor.getState().draft).toBe('draft b');
});

it('allows context changes while an independent file save remains tracked', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('saving draft');
  const pending = Promise.withResolvers<unknown>();
  invoke.mockImplementation((command: string) => command === 'save_development_file' ? pending.promise : Promise.resolve(null));
  const saving = useDevelopmentEditor.getState().save();
  expect(await chooseContext({ project_id: 'other', task_id: null, session_id: null })).toEqual({ ok: true, value: null });
  expect(useDevelopmentEditor.getState().saving).toBe(true);
  expect(invoke.mock.calls.some(([command]) => command === 'select_development_context')).toBe(true);
  pending.resolve({ path: 'a.ts', text: 'saving draft', hash: 'saved-hash', size: 12, reason: null });
  await saving;
  expect(useDevelopmentEditor.getState().file?.hash).toBe('saved-hash');
});

it('renames documents only in the filesystem workspace that changed', async () => {
  installFiles();
  for (const workspacePath of ['/one', '/two']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath, path: 'folder/a.ts', mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', 'folder/a.ts', workspacePath);
    useDevelopmentEditor.getState().edit(`draft ${workspacePath}`);
  }
  remapDevelopmentDocuments('p', 't', 'folder', 'renamed', '/one');
  expect(useDevelopmentEditor.getState().tabs.map(tab => [tab.workspacePath, tab.path])).toEqual([['/one', 'renamed/a.ts'], ['/two', 'folder/a.ts']]);
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'renamed/a.ts', '/one')].draft).toBe('draft /one');
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'folder/a.ts', '/two')].draft).toBe('draft /two');
});

it('preserves a clean document edited while its async close guard is settling', async () => {
  installFiles();
  openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path: 'a.ts', mode: 'file', source: 'worktree' });
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  const id = documentId('p', 't', 'a.ts', '/repo');
  const closing = closeDevelopmentDocuments(useDevelopmentEditor.getState().tabs.map(tab => tab.id));
  useDevelopmentEditor.getState().edit('new unsaved draft', id);
  expect(await closing).toBe(false);
  expect(useDevelopmentEditor.getState().documents[id].draft).toBe('new unsaved draft');
  expect(confirmDiscard).not.toHaveBeenCalled();
});

it('saves every affected draft before allowing batch close', async () => {
  installFiles();
  for (const path of ['a.ts', 'b.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
    useDevelopmentEditor.getState().edit(`draft ${path}`);
  }
  const a = Promise.withResolvers<unknown>(), b = Promise.withResolvers<unknown>();
  invoke.mockImplementation((command: string, args: { input: { path: string } }) => command === 'save_development_file' ? (args.input.path === 'a.ts' ? a.promise : b.promise) : Promise.resolve(null));
  render(<ModalHost />);
  const ids = useDevelopmentEditor.getState().tabs.map(tab => tab.id);
  const closing = closeDevelopmentDocuments(ids);
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await waitFor(() => expect(invoke.mock.calls.filter(([command]) => command === 'save_development_file')).toHaveLength(2));
  expect(invoke).toHaveBeenCalledWith('save_development_file', { input: { project_id: 'p', task_id: 't', path: 'a.ts', expected_hash: 'initial-hash', text: 'draft a.ts' } });
  expect(invoke).toHaveBeenCalledWith('save_development_file', { input: { project_id: 'p', task_id: 't', path: 'b.ts', expected_hash: 'initial-hash', text: 'draft b.ts' } });
  await act(async () => { a.resolve({ path: 'a.ts', text: 'draft a.ts', hash: 'saved-a', size: 10, reason: null }); });
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.id)).toEqual(ids);
  await act(async () => {
    b.resolve({ path: 'b.ts', text: 'draft b.ts', hash: 'saved-b', size: 10, reason: null });
    expect(await closing).toBe(true);
  });
  expect(useDevelopmentEditor.getState().tabs).toEqual([]);
  expect(useDevelopmentEditor.getState().documents).toEqual({});
  expect(confirmDiscard).not.toHaveBeenCalled();
});

it('keeps every tab and the failed draft when a batch save fails', async () => {
  installFiles();
  for (const path of ['a.ts', 'b.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
    useDevelopmentEditor.getState().edit(`draft ${path}`);
  }
  invoke.mockImplementation((command: string, args: { input: { path: string; text: string } }) => command === 'save_development_file'
    ? args.input.path === 'b.ts' ? Promise.reject('file_access_denied') : Promise.resolve({ path: args.input.path, text: args.input.text, hash: 'saved-a', size: 10, reason: null })
    : Promise.resolve(null));
  render(<ModalHost />);
  const ids = useDevelopmentEditor.getState().tabs.map(tab => tab.id);
  const closing = closeDevelopmentDocuments(ids);
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  expect(await closing).toBe(false);
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.id)).toEqual(ids);
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'a.ts', '/repo')]).toMatchObject({ draft: 'draft a.ts', file: { text: 'draft a.ts' } });
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'b.ts', '/repo')]).toMatchObject({ draft: 'draft b.ts', file: { text: 'base' }, error: 'file_access_denied', saving: false });
});

it('allows reload after saving the approved draft despite save-induced document replacement', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('saved draft');
  const disk = { path: 'a.ts', text: 'saved draft', hash: 'saved-hash', size: 11, reason: null };
  invoke.mockImplementation((command: string) => Promise.resolve(command === 'save_development_file' || command === 'read_development_file' ? disk : null));
  render(<ModalHost />);
  const reloading = useDevelopmentEditor.getState().reloadDisk();
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await reloading;
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_file')).toHaveLength(1);
  expect(invoke.mock.calls.filter(([command]) => command === 'read_development_file')).toHaveLength(2);
  expect(useDevelopmentEditor.getState()).toMatchObject({ draft: 'saved draft', file: disk, busy: false, saving: false });
});

it('does not reload after a failed save-and-continue request', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('keep my draft');
  invoke.mockRejectedValueOnce('file_access_denied');
  render(<ModalHost />);
  const reloading = useDevelopmentEditor.getState().reloadDisk();
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await reloading;
  expect(useDevelopmentEditor.getState()).toMatchObject({ draft: 'keep my draft', file: { text: 'base' }, error: 'file_access_denied' });
  expect(invoke.mock.calls.filter(([command]) => command === 'read_development_file')).toHaveLength(1);
});

it('refuses to close when typing changes during save, even if the approved text is restored', async () => {
  installFiles();
  openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path: 'a.ts', mode: 'file', source: 'worktree' });
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('approved draft');
  const pending = Promise.withResolvers<unknown>(); invoke.mockReturnValueOnce(pending.promise);
  render(<ModalHost />);
  const ids = useDevelopmentEditor.getState().tabs.map(tab => tab.id);
  const closing = closeDevelopmentDocuments(ids);
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await waitFor(() => expect(useDevelopmentEditor.getState().saving).toBe(true));
  useDevelopmentEditor.getState().edit('new typing');
  useDevelopmentEditor.getState().edit('approved draft');
  pending.resolve({ path: 'a.ts', text: 'approved draft', hash: 'saved', size: 14, reason: null });
  expect(await closing).toBe(false);
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.id)).toEqual(ids);
  expect(useDevelopmentEditor.getState().draft).toBe('approved draft');
});

it('keeps a clean affected document that becomes dirty while another document is saving', async () => {
  installFiles();
  for (const path of ['a.ts', 'b.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
  }
  const a = documentId('p', 't', 'a.ts', '/repo'), b = documentId('p', 't', 'b.ts', '/repo');
  useDevelopmentEditor.getState().edit('approved a', a);
  const pending = Promise.withResolvers<unknown>(); invoke.mockReturnValueOnce(pending.promise);
  render(<ModalHost />);
  const ids = useDevelopmentEditor.getState().tabs.map(tab => tab.id);
  const closing = closeDevelopmentDocuments(ids);
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await waitFor(() => expect(useDevelopmentEditor.getState().documents[a].saving).toBe(true));
  useDevelopmentEditor.getState().edit('new b draft', b);
  pending.resolve({ path: 'a.ts', text: 'approved a', hash: 'saved-a', size: 10, reason: null });
  expect(await closing).toBe(false);
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.id)).toEqual(ids);
  expect(useDevelopmentEditor.getState().documents[b].draft).toBe('new b draft');
});

it('rejects replaced document and account context while save-and-continue is pending', async () => {
  installFiles();
  openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path: 'a.ts', mode: 'file', source: 'worktree' });
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('old account draft');
  const pending = Promise.withResolvers<unknown>(); invoke.mockReturnValueOnce(pending.promise);
  render(<ModalHost />);
  const closing = closeDevelopmentDocuments(useDevelopmentEditor.getState().tabs.map(tab => tab.id));
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await waitFor(() => expect(useDevelopmentEditor.getState().saving).toBe(true));
  useDevelopment.getState().reset();
  openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path: 'a.ts', mode: 'file', source: 'worktree' });
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('new account draft');
  pending.resolve({ path: 'a.ts', text: 'old account draft', hash: 'old-saved', size: 17, reason: null });
  expect(await closing).toBe(false);
  expect(useDevelopmentEditor.getState().tabs).toHaveLength(1);
  expect(useDevelopmentEditor.getState().draft).toBe('new account draft');
  expect(useDevelopmentEditor.getState().file?.text).toBe('base');
});

it('rejects a changed selection while saving through the exported document guard', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('approved draft');
  const pending = Promise.withResolvers<unknown>(); invoke.mockReturnValueOnce(pending.promise);
  render(<ModalHost />);
  const guarding = guardDevelopmentDocuments('p', 't', ['a.ts']);
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await waitFor(() => expect(useDevelopmentEditor.getState().saving).toBe(true));
  useDevelopment.setState({ selection: { project_id: 'other', task_id: null, session_id: null } });
  pending.resolve({ path: 'a.ts', text: 'approved draft', hash: 'saved', size: 14, reason: null });
  expect(await guarding).toBe(false);
  expect(useDevelopmentEditor.getState().draft).toBe('approved draft');
});

it('cancels editing atomically after save or discard and preserves conflict evidence', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  const id = documentId('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('save before leaving');
  invoke.mockResolvedValueOnce({ path: 'a.ts', text: 'save before leaving', hash: 'saved', size: 19, reason: null });
  render(<ModalHost />);
  const saving = cancelDevelopmentDocumentEdit(id);
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  expect(await saving).toBe(true);
  expect(useDevelopmentEditor.getState()).toMatchObject({ draft: 'save before leaving', file: { text: 'save before leaving', hash: 'saved' } });
  useDevelopmentEditor.getState().edit('conflicting draft');
  const document = useDevelopmentEditor.getState().documents[id];
  useDevelopmentEditor.setState({ documents: { [id]: { ...document, error: 'file_changed', disk: { path: 'a.ts', text: 'external', hash: 'external', size: 8, reason: null } } } });
  const discarding = cancelDevelopmentDocumentEdit(id);
  expect(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' })).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.discardAndContinue' }));
  expect(await discarding).toBe(true);
  expect(useDevelopmentEditor.getState().documents[id]).toMatchObject({ draft: 'save before leaving', error: 'file_changed', disk: { hash: 'external' } });
});

it('keeps restored documents unsaved until disk revalidation and allows explicit discard', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('restored draft');
  const id = documentId('p', 't', 'a.ts'), document = useDevelopmentEditor.getState().documents[id];
  useDevelopmentEditor.setState({ documents: { [id]: { ...document, restored: true } } });
  render(<ModalHost />);
  const guarded = guardDevelopmentDocuments('p', 't', ['a.ts']);
  expect(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' })).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.discardAndContinue' }));
  expect(await guarded).toBe(true);
  expect(useDevelopmentEditor.getState().documents[id].draft).toBe('restored draft');
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_file')).toBe(false);
});

it('allows the whole-draft guard to continue after its own successful save', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('approved draft');
  invoke.mockResolvedValueOnce({ path: 'a.ts', text: 'approved draft', hash: 'saved', size: 14, reason: null });
  render(<ModalHost />);
  const discarding = discardDevelopmentDraft();
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  expect(await discarding).toBe(true);
  expect(useDevelopmentEditor.getState()).toMatchObject({ draft: 'approved draft', file: { text: 'approved draft' } });
});

it('keeps an unsaved selection out of the conversation when save-and-add fails', async () => {
  installFiles();
  const reference = vi.fn();
  render(<FileEditor projectId="p" taskId="t" path="a.ts" onReference={reference} />);
  const editor = await screen.findByTestId('dev-file-editor') as HTMLTextAreaElement;
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.editFile' }));
  fireEvent.change(editor, { target: { value: 'keep this draft' } });
  editor.setSelectionRange(0, 4); fireEvent.select(editor);
  invoke.mockRejectedValueOnce('file_access_denied');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.saveAndAdd' }));
  await waitFor(() => expect(useDevelopmentEditor.getState().saving).toBe(false));
  expect(reference).not.toHaveBeenCalled();
  expect(editor.value).toBe('keep this draft');
  expect(useDevelopmentEditor.getState().error).toBe('file_access_denied');
});

it('returns to the same draft when cancel-edit is dismissed and rejects edits made during consent', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts');
  const id = documentId('p', 't', 'a.ts');
  useDevelopmentEditor.getState().edit('keep draft');
  render(<ModalHost />);
  const cancelled = cancelDevelopmentDocumentEdit(id);
  const dialog = await screen.findByRole('dialog', { name: 'workbenchFiles.unsavedTitle' });
  fireEvent.keyDown(dialog, { key: 'Escape' });
  expect(await cancelled).toBe(false);
  expect(useDevelopmentEditor.getState().draft).toBe('keep draft');
  const stale = cancelDevelopmentDocumentEdit(id);
  await screen.findByRole('button', { name: 'workbenchFiles.discardAndContinue' });
  useDevelopmentEditor.getState().edit('newer draft');
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.discardAndContinue' }));
  expect(await stale).toBe(false);
  expect(useDevelopmentEditor.getState().draft).toBe('newer draft');
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_file')).toBe(false);
});

it('retains conflict evidence when save-and-continue detects changed disk', async () => {
  installFiles();
  openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path: 'a.ts', mode: 'file', source: 'worktree' });
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('conflicting draft');
  invoke.mockImplementation((command: string) => command === 'save_development_file' ? Promise.reject('file_changed') : Promise.resolve({ path: 'a.ts', text: 'external text', hash: 'external', size: 13, reason: null }));
  render(<ModalHost />);
  const closing = closeDevelopmentDocuments(useDevelopmentEditor.getState().tabs.map(tab => tab.id));
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  expect(await closing).toBe(false);
  expect(useDevelopmentEditor.getState().tabs).toHaveLength(1);
  expect(useDevelopmentEditor.getState()).toMatchObject({ draft: 'conflicting draft', file: { text: 'base' }, error: 'file_changed', disk: { text: 'external text', hash: 'external' } });
});

it('blocks pending mutations and keeps save disabled for explicitly allowed reconciliation', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('locked draft');
  const key = developmentDraftKey('delivery', 'p', 't', '/repo', 'git-hunk', 'a.ts');
  useDevelopmentDrafts.setState({ entries: { [key]: { kind: 'delivery', baseline: null, value: { operation: { action: 'restore_hunk', path: 'a.ts', status: 'unknown' } } } } });
  expect(await guardDevelopmentDocuments('p', 't', ['a.ts'], '/repo')).toBe(false);
  render(<ModalHost />);
  const reconcile = guardDevelopmentDocuments('p', 't', ['a.ts'], '/repo', true);
  expect(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' })).toHaveProperty('disabled', true);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.discardAndContinue' }));
  expect(await reconcile).toBe(true);
  expect(useDevelopmentEditor.getState().draft).toBe('locked draft');
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_file')).toBe(false);
});

it('refreshes delete authority after guard saves before deleting the file', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('save then delete');
  let version = 'before-save';
  invoke.mockImplementation(async (command: string) => {
    if (command === 'list_development_files') return { entries: [{ path: 'a.ts', name: 'a.ts', kind: 'file', ignored: false, size: 4 }], truncated: false };
    if (command === 'inspect_development_entry') return { path: 'a.ts', kind: 'file', version };
    if (command === 'save_development_file') { version = 'after-save'; return { path: 'a.ts', text: 'save then delete', hash: 'saved', size: 16, reason: null }; }
    if (command === 'mutate_development_entry') return { path: 'a.ts', kind: 'file' };
    return null;
  });
  const changed = vi.fn();
  render(<><Files projectId="p" taskId="t" workspacePath="/repo" onOpenFile={() => {}} onMutation={changed} /><ModalHost /></>);
  fireEvent.keyDown(await screen.findByTestId('dev-file-item-a.ts'), { key: 'Delete' });
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.delete' }));
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await waitFor(() => expect(changed).toHaveBeenCalledWith({ action: 'delete', path: 'a.ts', destination: null }));
  expect(invoke).toHaveBeenCalledWith('mutate_development_entry', { input: { project_id: 'p', task_id: 't', path: 'a.ts', expected_version: 'after-save', action: 'delete', destination: null, trust_directory: true } });
  expect(invoke.mock.calls.filter(([command]) => command === 'inspect_development_entry')).toHaveLength(2);
});

it('rejects a directory delete when saving a child also admits an external addition to the snapshot', async () => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'folder/a.ts', '/repo');
  useDevelopmentEditor.getState().edit('saved child draft');
  let version = 'reviewed-directory', externalAdded = false;
  invoke.mockImplementation(async (command: string, args: { input?: { path: string } }) => {
    if (command === 'list_development_files') return { entries: args.input?.path ? [
      { path: 'folder/a.ts', name: 'a.ts', kind: 'file', ignored: false, size: 4 },
      ...(externalAdded ? [{ path: 'folder/external.ts', name: 'external.ts', kind: 'file', ignored: false, size: 4 }] : []),
    ] : [{ path: 'folder', name: 'folder', kind: 'directory', ignored: false, size: 0 }], truncated: false };
    if (command === 'inspect_development_entry') return { path: 'folder', kind: 'directory', version };
    if (command === 'save_development_file') {
      externalAdded = true; version = 'saved-child-and-new-external-child';
      return { path: 'folder/a.ts', text: 'saved child draft', hash: 'saved', size: 17, reason: null };
    }
    if (command === 'mutate_development_entry') return { path: 'folder', kind: 'directory' };
    return null;
  });
  const changed = vi.fn();
  render(<><Files projectId="p" taskId="t" workspacePath="/repo" onOpenFile={() => {}} onMutation={changed} /><ModalHost /></>);
  fireEvent.keyDown(await screen.findByTestId('dev-file-item-folder'), { key: 'Delete' });
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.delete' }));
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await screen.findByRole('alert');
  expect(externalAdded).toBe(true);
  expect(invoke.mock.calls.filter(([command]) => command === 'mutate_development_entry')).toEqual([]);
  expect(changed).not.toHaveBeenCalled();
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'folder/a.ts', '/repo')]).toMatchObject({ draft: 'saved child draft', file: { hash: 'saved' } });

  // Only a new delete gesture may authorize the changed directory snapshot.
  fireEvent.keyDown(screen.getByTestId('dev-file-item-folder'), { key: 'Delete' });
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.delete' }));
  await waitFor(() => expect(changed).toHaveBeenCalledWith({ action: 'delete', path: 'folder', destination: null }));
  expect(invoke).toHaveBeenCalledWith('mutate_development_entry', { input: { project_id: 'p', task_id: 't', path: 'folder', expected_version: 'saved-child-and-new-external-child', action: 'delete', destination: null, trust_directory: true } });
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_file')).toHaveLength(1);
});

it.each(['unchanged', 'changed'])('allows a restored draft to retry disk reads and preserves it when disk is %s', async state => {
  installFiles();
  await useDevelopmentEditor.getState().open('p', 't', 'a.ts', '/repo');
  const id = documentId('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.getState().edit('recovered unsaved draft', id);
  const document = useDevelopmentEditor.getState().documents[id];
  useDevelopmentEditor.setState({ documents: { [id]: { ...document, restored: true } } });
  let reads = 0;
  invoke.mockClear();
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_file') {
      if (++reads <= 2) throw 'file_access_denied';
      return { path: 'a.ts', text: state === 'changed' ? 'external text' : 'base', hash: state === 'changed' ? 'external-hash' : 'initial-hash', size: 4, reason: null };
    }
    return null;
  });
  render(<><FileEditor projectId="p" taskId="t" path="a.ts" workspacePath="/repo" editing /><ModalHost /></>);
  await screen.findByRole('alert');
  expect(screen.getByTestId('dev-file-save')).toHaveProperty('disabled', true);
  await useDevelopmentEditor.getState().save(id);
  for (const count of [2, 3]) {
    fireEvent.keyDown(screen.getByRole('button', { name: 'workbenchFiles.fileActions' }), { key: 'ArrowDown' });
    const reload = await screen.findByRole('menuitem', { name: 'workbenchFiles.reloadDisk' });
    expect(reload.getAttribute('aria-disabled')).not.toBe('true');
    fireEvent.click(reload);
    await waitFor(() => expect(reads).toBe(count));
    await waitFor(() => expect(useDevelopmentEditor.getState().documents[id].busy).toBe(false));
    expect(useDevelopmentEditor.getState().documents[id].draft).toBe('recovered unsaved draft');
    expect(screen.queryByRole('dialog', { name: 'workbenchFiles.unsavedTitle' })).toBeNull();
    if (count === 2) {
      expect(useDevelopmentEditor.getState().documents[id].restored).toBe(true);
      expect(screen.getByTestId('dev-file-save')).toHaveProperty('disabled', true);
    }
  }
  expect(useDevelopmentEditor.getState().documents[id].restored).toBe(false);
  if (state === 'changed') {
    expect(screen.getByTestId('dev-file-conflict')).toBeTruthy();
    expect(useDevelopmentEditor.getState().documents[id].disk?.text).toBe('external text');
    expect(screen.getByTestId('dev-file-save')).toHaveProperty('disabled', true);
  } else expect(screen.getByTestId('dev-file-save')).toHaveProperty('disabled', false);
  expect(invoke.mock.calls.filter(([command]) => command === 'save_development_file')).toEqual([]);
});

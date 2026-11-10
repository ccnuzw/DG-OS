import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ProjectReview } from '../src/components/development/ProjectReview';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentEditor, documentId, openDevelopmentDocument, closeDevelopmentDocuments, guardDevelopmentDraft, remapDevelopmentDocuments, invalidateDevelopmentDocuments } from '../src/stores/developmentEditor';
import { chooseContext } from '../src/stores/developmentSelection';
import { useDevelopmentRuntime } from '../src/stores/developmentRuntime';
import { FileEditor } from '../src/components/development/FileEditor';
const { invoke, confirmDiscard } = vi.hoisted(() => ({ invoke: vi.fn(), confirmDiscard: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/workspace/closeCoordinator', () => ({ confirmDiscard }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
afterEach(() => { cleanup(); useDevelopment.getState().reset(); invoke.mockReset(); confirmDiscard.mockReset(); });
function installFiles() {
  invoke.mockImplementation((command: string, args: { input?: { path: string } }) => {
    if (command === 'list_development_files') return Promise.resolve({ entries: ['a.ts', 'b.ts'].map(path => ({ path, name: path, kind: 'file', size: 4, ignored: false })), truncated: false });
    if (command === 'read_development_file') return Promise.resolve({ path: args.input?.path, text: 'base', hash: 'initial-hash', size: 4, reason: null });
    return Promise.resolve(null);
  });
}
it('retains independent file drafts through filtering and project changes without asking to discard', async () => {
  installFiles(); confirmDiscard.mockResolvedValue(false);
  render(<ProjectReview projectId="p" taskId="t" />);
  fireEvent.click(await screen.findByTestId('dev-file-item-a.ts'));
  fireEvent.change(await screen.findByTestId('dev-file-editor'), { target: { value: 'draft a' } });
  fireEvent.click(screen.getByTestId('dev-file-item-b.ts'));
  await waitFor(() => expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('base'));
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
it('keeps the same file editor and draft mounted while reviewing changes', async () => {
  installFiles();
  render(<ProjectReview projectId="p" taskId="t" />);
  fireEvent.click(await screen.findByTestId('dev-file-item-a.ts'));
  const editor = await screen.findByTestId('dev-file-editor') as HTMLTextAreaElement;
  fireEvent.change(editor, { target: { value: 'unsaved draft' } });
  const changes = screen.getByTestId('dev-review-tab-changes');
  act(() => { changes.focus(); });
  expect(screen.getByTestId('dev-review-tab-files').getAttribute('aria-selected')).toBe('true');
  fireEvent.keyDown(changes, { key: 'Enter' });
  await waitFor(() => expect(changes.getAttribute('aria-selected')).toBe('true'));
  expect(editor.closest('[role="tabpanel"]')?.hasAttribute('hidden')).toBe(true);
  expect(confirmDiscard).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByTestId('dev-review-tab-files'), { key: 'Enter' });
  expect(screen.getByTestId('dev-file-editor')).toBe(editor);
  expect(editor.value).toBe('unsaved draft');
  expect(invoke.mock.calls.filter(([command]) => command === 'read_development_file')).toHaveLength(1);
});
it('retains a conflict draft, compares reread disk and requires explicit discard to reload', async () => {
  installFiles();
  render(<ProjectReview projectId="p" taskId="t" />);
  fireEvent.click(await screen.findByTestId('dev-file-item-a.ts'));
  fireEvent.change(await screen.findByTestId('dev-file-editor'), { target: { value: 'my draft' } });
  invoke.mockImplementation((command: string) => command === 'save_development_file' ? Promise.reject('file_changed') : Promise.resolve({ path: 'a.ts', text: 'external edit', hash: 'disk-hash', size: 13, reason: null }));
  fireEvent.click(screen.getByTestId('dev-file-save'));
  await screen.findByText('external edit');
  expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('my draft');
  expect(invoke).toHaveBeenCalledWith('save_development_file', { input: { project_id: 'p', task_id: 't', path: 'a.ts', expected_hash: 'initial-hash', text: 'my draft' } });
  confirmDiscard.mockResolvedValueOnce(false);
  fireEvent.click(screen.getByTestId('dev-file-reload'));
  await waitFor(() => expect(confirmDiscard).toHaveBeenCalledTimes(1));
  expect(useDevelopmentEditor.getState().draft).toBe('my draft');
  confirmDiscard.mockResolvedValueOnce(true);
  fireEvent.click(screen.getByTestId('dev-file-reload'));
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
  fireEvent.change(editor, { target: { value: 'changed\nsecond\nthird' } });
  expect(editor.selectionStart).not.toBe(8);
  expect(invoke.mock.calls.some(([command]) => command === 'save_development_file')).toBe(false);
});

it('saves the current draft once after composition ends and ignores repeating save keys', async () => {
  installFiles();
  render(<FileEditor projectId="p" taskId="t" path="a.ts" />);
  const editor = await screen.findByTestId('dev-file-editor');
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

it('references saved selected lines and refuses an unsaved selection', async () => {
  invoke.mockResolvedValue({ path: 'a.ts', text: 'first\nsecond\nthird', hash: 'disk-version', size: 18, reason: null });
  const reference = vi.fn();
  render(<FileEditor projectId="p" taskId="t" path="a.ts" onReference={reference} />);
  const editor = await screen.findByTestId('dev-file-editor') as HTMLTextAreaElement;
  editor.setSelectionRange(6, 13); fireEvent.select(editor);
  fireEvent.click(screen.getByRole('button', { name: 'workbenchFiles.referenceSelection' }));
  expect(reference).toHaveBeenCalledWith({ project_id: 'p', task_id: 't', path: 'a.ts', kind: 'selection', expected_hash: 'disk-version', start_line: 2, end_line: 2 });
  fireEvent.change(editor, { target: { value: 'unsaved\nsecond\nthird' } });
  editor.setSelectionRange(0, 7); fireEvent.select(editor);
  expect(screen.getByRole('button', { name: 'workbenchFiles.referenceSelection' })).toHaveProperty('disabled', true);
  expect(screen.getByRole('button', { name: 'workbenchFiles.referenceFile' })).toHaveProperty('disabled', true);
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
  confirmDiscard.mockResolvedValueOnce(false);
  expect(await closeDevelopmentDocuments(ids)).toBe(false);
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.id)).toEqual(ids);
  confirmDiscard.mockResolvedValueOnce(true);
  expect(await closeDevelopmentDocuments(ids)).toBe(true);
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
  confirmDiscard.mockResolvedValue(false);
  expect(await guardDevelopmentDraft()).toBe(false);
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

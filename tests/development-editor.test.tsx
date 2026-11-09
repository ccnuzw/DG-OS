import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { ProjectReview } from '../src/components/development/ProjectReview';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentEditor } from '../src/stores/developmentEditor';
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
it('cancelling file, search and context changes keeps the draft while code views coexist', async () => {
  installFiles(); confirmDiscard.mockResolvedValue(false);
  render(<ProjectReview projectId="p" taskId="t" />);
  fireEvent.click(await screen.findByTestId('dev-file-item-a.ts'));
  fireEvent.change(await screen.findByTestId('dev-file-editor'), { target: { value: 'unsaved draft' } });
  fireEvent.click(screen.getByTestId('dev-file-item-b.ts'));
  await waitFor(() => expect(confirmDiscard).toHaveBeenCalledTimes(1));
  fireEvent.change(screen.getByTestId('dev-file-search'), { target: { value: 'b' } });
  await waitFor(() => expect(confirmDiscard).toHaveBeenCalledTimes(2));
  fireEvent.keyDown(screen.getByTestId('dev-review-tab-changes'), { key: 'Enter' });
  expect(confirmDiscard).toHaveBeenCalledTimes(2);
  fireEvent.keyDown(screen.getByTestId('dev-review-tab-files'), { key: 'Enter' });
  await act(async () => { expect((await chooseContext({ project_id: 'other', task_id: null, session_id: null })).ok).toBe(false); });
  expect((screen.getByTestId('dev-file-editor') as HTMLTextAreaElement).value).toBe('unsaved draft');
  expect((screen.getByTestId('dev-file-search') as HTMLInputElement).value).toBe('');
  expect(invoke.mock.calls.some(([command]) => command === 'select_development_context')).toBe(false);
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

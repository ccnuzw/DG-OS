import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { Modal } from '@dgos/ui';
import { FileWorkspace } from '../src/components/development/FileWorkspace';
import { GitFileDiff, parseUnifiedDiff } from '../src/components/development/GitFileDiff';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts, developmentDraftKey } from '../src/stores/developmentDrafts';
import { useDevelopmentEditor, documentId, openDevelopmentDocument } from '../src/stores/developmentEditor';
const { invoke, confirmDiscard } = vi.hoisted(() => ({ invoke: vi.fn(), confirmDiscard: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/workspace/closeCoordinator', () => ({ confirmDiscard }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ t: (key: string) => key }), t: (key: string) => key, formatDateTime: String }));
const project = { id: 'p', owner: 'alice', name: 'Project', path: '/repo', directory_identity: 'root', archived: false, created_at: 1, last_opened_at: null };
const text = 'diff --git a/a.ts b/a.ts\n--- a/a.ts\n+++ b/a.ts\n@@ -1 +1 @@\n-old\n+new\n@@ -9 +9 @@\n-before\n+after\n';
let diff = { path: 'a.ts', before: 'old', after: 'new', diff: text, diff_hash: 'host-raw-diff-hash', reason: null };
const originalLoad = useDevelopment.getState().load;
beforeEach(() => {
  localStorage.clear(); useDevelopment.getState().reset(); useDevelopmentDrafts.getState().clear(); useDevelopment.setState({ projects: [project] });
  diff = { path: 'a.ts', before: 'old', after: 'new', diff: text, diff_hash: 'host-raw-diff-hash', reason: null };
  invoke.mockImplementation(async (command: string, args: { input?: { path?: string }; name?: string }) => {
    if (command === 'read_development_file') return { path: args.input?.path, text: args.input?.path?.endsWith('.md') ? '| Name | Value |\n| --- | --- |\n| example | safe |' : 'base', hash: 'file-hash', size: 4, reason: null };
    if (command === 'read_development_diff' || command === 'read_development_project_diff') return { ...diff };
    if (command === 'read_development_git') return { path: '/repo', review_token: 'fresh-review' };
    if (command === 'read_development_workspaces') return { operations: [], workspaces: [] };
    return null;
  });
});
afterEach(() => { cleanup(); useDevelopment.getState().reset(); useDevelopment.setState({ load: originalLoad }); invoke.mockReset(); confirmDiscard.mockReset(); vi.restoreAllMocks(); });
it('shares file and diff tabs, retains two drafts, and cancels batch close', async () => {
  const onClose = vi.fn(), onOpenFile = vi.fn();
  const view = render(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={{ path: 'a.ts', mode: 'file', nonce: 1 }} onClose={onClose} onOpenFile={onOpenFile} />);
  fireEvent.change(await screen.findByTestId('dev-file-editor'), { target: { value: 'draft a' } });
  view.rerender(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={{ path: 'b.ts', mode: 'file', nonce: 2 }} onClose={onClose} onOpenFile={onOpenFile} />);
  await waitFor(() => expect(screen.getAllByTestId('dev-file-editor')).toHaveLength(2));
  fireEvent.change(screen.getAllByTestId('dev-file-editor')[1], { target: { value: 'draft b' } });
  view.rerender(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={{ path: 'a.ts', mode: 'diff', nonce: 3 }} onClose={onClose} onOpenFile={onOpenFile} />);
  await screen.findByTestId('dev-file-diff');
  expect(screen.getAllByRole('tab')).toHaveLength(3);
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'a.ts', '/repo')].draft).toBe('draft a');
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'b.ts', '/repo')].draft).toBe('draft b');
  confirmDiscard.mockResolvedValue(false);
  fireEvent.keyDown(screen.getByRole('button', { name: 'fileWorkspace.tabActions' }), { key: 'Enter' });
  fireEvent.click(await screen.findByRole('menuitem', { name: 'fileWorkspace.closeAll' }));
  await waitFor(() => expect(confirmDiscard).toHaveBeenCalledTimes(1));
  expect(screen.getAllByRole('tab')).toHaveLength(3); expect(onClose).not.toHaveBeenCalled();
});
it('closes only scoped tabs to the right of a context target without writing files', async () => {
  for (const path of ['src/index.ts', 'test/index.ts', 'right.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
  }
  openDevelopmentDocument({ projectId: 'p', taskId: 'other', workspacePath: '/other', path: 'foreign.ts', mode: 'file', source: 'worktree' });
  const onClose = vi.fn();
  render(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={null} onClose={onClose} onOpenFile={() => {}} />);
  expect(screen.getByRole('tab', { name: 'src/index.ts' }).getAttribute('title')).toBe('src/index.ts');
  expect(screen.getByRole('tab', { name: 'test/index.ts' })).toBeTruthy();
  fireEvent.contextMenu(screen.getByRole('tab', { name: 'test/index.ts' }), { clientX: 20, clientY: 20 });
  fireEvent.click(await screen.findByRole('menuitem', { name: 'fileWorkspace.closeRight' }));
  await waitFor(() => expect(screen.getAllByRole('tab')).toHaveLength(2));
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.path)).toEqual(['src/index.ts', 'test/index.ts', 'foreign.ts']);
  expect(onClose).not.toHaveBeenCalled();
  expect(confirmDiscard).not.toHaveBeenCalled();
  expect(invoke.mock.calls.every(([command]) => command === 'read_development_file')).toBe(true);
  await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'test/index.ts' })));
});

it('navigates tabs by keyboard and closes other tabs through the accessible menu', async () => {
  for (const path of ['a.ts', 'b.ts', 'c.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
  }
  render(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={null} onClose={() => {}} onOpenFile={() => {}} />);
  fireEvent.keyDown(screen.getByRole('tab', { name: 'c.ts' }), { key: 'Home' });
  expect(screen.getByRole('tab', { name: 'a.ts' }).getAttribute('aria-selected')).toBe('true');
  fireEvent.keyDown(screen.getByRole('tab', { name: 'a.ts' }), { key: 'ArrowLeft' });
  expect(screen.getByRole('tab', { name: 'c.ts' }).getAttribute('aria-selected')).toBe('true');
  fireEvent.keyDown(screen.getByRole('tab', { name: 'c.ts' }), { key: 'Home' });
  fireEvent.keyDown(screen.getByRole('tab', { name: 'a.ts' }), { key: 'ArrowRight' });
  expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'b.ts' }));
  fireEvent.keyDown(screen.getByRole('button', { name: 'fileWorkspace.tabActions' }), { key: 'Enter' });
  fireEvent.click(await screen.findByRole('menuitem', { name: 'fileWorkspace.closeOthers' }));
  await waitFor(() => expect(screen.getAllByRole('tab')).toHaveLength(1));
  fireEvent.keyDown(screen.getByRole('tab', { name: 'b.ts' }), { key: 'F10', shiftKey: true });
  expect((await screen.findByRole('menuitem', { name: 'fileWorkspace.closeOthers' })).getAttribute('aria-disabled')).toBe('true');
  expect(screen.getByRole('menuitem', { name: 'fileWorkspace.closeRight' }).getAttribute('aria-disabled')).toBe('true');
  fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
  fireEvent.keyDown(screen.getByRole('tab', { name: 'b.ts' }), { key: 'Delete' });
  await waitFor(() => expect(useDevelopmentEditor.getState().tabs).toEqual([]));
});

it('locks repeated dirty closes, keeps the draft on cancel, and restores tab focus', async () => {
  const pending = Promise.withResolvers<boolean>();
  confirmDiscard.mockReturnValue(pending.promise);
  const onClose = vi.fn();
  render(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={{ path: 'a.ts', mode: 'file', nonce: 1 }} onClose={onClose} onOpenFile={() => {}} />);
  fireEvent.change(await screen.findByTestId('dev-file-editor'), { target: { value: 'draft' } });
  const tab = screen.getByRole('tab', { name: 'a.ts, direct.unsaved' });
  fireEvent.keyDown(tab, { key: 'Delete' });
  fireEvent.keyDown(tab, { key: 'Delete' });
  await waitFor(() => expect(confirmDiscard).toHaveBeenCalledTimes(1));
  await act(async () => { pending.resolve(false); });
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'a.ts', '/repo')].draft).toBe('draft');
  expect(onClose).not.toHaveBeenCalled();
  await waitFor(() => expect(document.activeElement).toBe(tab));
});

it('protects an entire close batch while saves or unknown mutations are pending', async () => {
  for (const path of ['a.ts', 'b.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
  }
  const id = documentId('p', 't', 'a.ts', '/repo');
  useDevelopmentEditor.setState(state => ({ documents: { ...state.documents, [id]: { ...state.documents[id], saving: true } } }));
  const onClose = vi.fn();
  render(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={null} onClose={onClose} onOpenFile={() => {}} />);
  fireEvent.click(screen.getByRole('button', { name: 'fileWorkspace.closePanel' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'fileWorkspace.closePanel' })).toHaveProperty('disabled', false));
  expect(screen.getAllByRole('tab')).toHaveLength(2);
  act(() => {
    useDevelopmentEditor.setState(state => ({ documents: { ...state.documents, [id]: { ...state.documents[id], saving: false } } }));
    useDevelopmentDrafts.setState({ entries: { [developmentDraftKey('delivery', 'p', 't')]: { kind: 'delivery', baseline: null, value: { operation: { action: 'delete', status: 'unknown', paths: ['a.ts'] } } } } });
  });
  fireEvent.contextMenu(screen.getByRole('tablist'), { clientX: 20, clientY: 20 });
  fireEvent.click(await screen.findByRole('menuitem', { name: 'fileWorkspace.closeAll' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'fileWorkspace.closePanel' })).toHaveProperty('disabled', false));
  expect(screen.getAllByRole('tab')).toHaveLength(2);
  expect(onClose).not.toHaveBeenCalled();
  expect(confirmDiscard).not.toHaveBeenCalled();
  expect(invoke.mock.calls.every(([command]) => command === 'read_development_file')).toBe(true);
});

it('does not finish an old account close after discard approval arrives', async () => {
  const pending = Promise.withResolvers<boolean>();
  confirmDiscard.mockReturnValue(pending.promise);
  const onClose = vi.fn();
  render(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={{ path: 'a.ts', mode: 'file', nonce: 1 }} onClose={onClose} onOpenFile={() => {}} />);
  fireEvent.change(await screen.findByTestId('dev-file-editor'), { target: { value: 'private draft' } });
  fireEvent.click(screen.getByRole('button', { name: 'fileWorkspace.closePanel' }));
  await waitFor(() => expect(confirmDiscard).toHaveBeenCalledTimes(1));
  act(() => { useDevelopment.getState().reset(); });
  await act(async () => { pending.resolve(true); });
  expect(onClose).not.toHaveBeenCalled();
  expect(useDevelopmentEditor.getState().tabs).toEqual([]);
  expect(useDevelopmentEditor.getState().documents).toEqual({});
});
it('closes all scoped documents from the empty bar while retaining another workspace', async () => {
  for (const path of ['a.ts', 'b.ts']) {
    openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/repo', path, mode: 'file', source: 'worktree' });
    await useDevelopmentEditor.getState().open('p', 't', path, '/repo');
  }
  openDevelopmentDocument({ projectId: 'p', taskId: 't', workspacePath: '/other', path: 'foreign.ts', mode: 'file', source: 'worktree' });
  const onClose = vi.fn();
  render(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={null} onClose={onClose} onOpenFile={() => {}} />);
  fireEvent.contextMenu(screen.getByRole('tablist'), { clientX: 20, clientY: 20 });
  fireEvent.click(await screen.findByRole('menuitem', { name: 'fileWorkspace.closeAll' }));
  await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
  expect(useDevelopmentEditor.getState().tabs.map(tab => tab.workspacePath)).toEqual(['/other']);
  expect(useDevelopmentEditor.getState().documents).toEqual({});
  expect(invoke.mock.calls.every(([command]) => command === 'read_development_file')).toBe(true);
});
it('renders Markdown tables safely by default and exposes editable source', async () => {
  render(<FileWorkspace projectId="p" taskId="t" workspacePath="/repo" target={{ path: 'README.md', mode: 'file', nonce: 1 }} onClose={() => {}} onOpenFile={() => {}} />);
  const table = await screen.findByRole('table');
  expect(within(table).getByText('example')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'direct.source' }));
  expect(screen.getByTestId('dev-file-editor').closest('[hidden]')).toBeNull();
});
it('counts independent hunks and assigns old/new line numbers correctly', () => {
  const rows = parseUnifiedDiff(text);
  expect(rows.filter(row => row.kind === 'hunk')).toHaveLength(2);
  expect(rows.find(row => row.text === '-old')).toMatchObject({ before: 1, after: null, hunk: 0 });
  expect(rows.find(row => row.text === '+after')).toMatchObject({ before: null, after: 9, hunk: 1 });
});
it('keeps review state for unchanged content and clears feedback after content changes', async () => {
  render(<GitFileDiff projectId="p" taskId="t" workspacePath="/repo" path="a.ts" source="worktree" onSourceChange={() => {}} onOpenFile={() => {}} />);
  const keep = await screen.findByRole('button', { name: 'fileWorkspace.keep' });
  await waitFor(() => expect(keep).toHaveProperty('disabled', false));
  fireEvent.click(keep); expect(screen.getByRole('button', { name: 'fileWorkspace.reviewed' }).getAttribute('aria-pressed')).toBe('true');
  fireEvent.click(screen.getByRole('button', { name: 'review.refresh' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'fileWorkspace.reviewed' }).getAttribute('aria-pressed')).toBe('true'));
  diff = { ...diff, diff: `${text}+changed\n`, diff_hash: 'changed-host-hash' };
  fireEvent.click(screen.getByRole('button', { name: 'review.refresh' }));
  await waitFor(() => expect(screen.getByRole('button', { name: 'fileWorkspace.keep' })).toHaveProperty('disabled', false));
  expect(screen.getByRole('button', { name: 'fileWorkspace.keep' }).getAttribute('aria-pressed')).toBe('false');
  expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git')).toBe(false);
});
it('dispatches selected hunk undo with the raw host hash and admitted review token', async () => {
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  useDevelopment.setState({ load: vi.fn().mockResolvedValue(true) });
  invoke.mockImplementation(async (command: string) => {
    if (command === 'read_development_diff') return { ...diff };
    if (command === 'read_development_git') return { path: '/repo', review_token: 'fresh-review' };
    if (command === 'read_development_workspaces') return { operations: [], workspaces: [] };
    if (command === 'execute_development_git') return { status: 'succeeded', details: {} };
    return null;
  });
  render(<GitFileDiff projectId="p" taskId="t" workspacePath="/repo" path="a.ts" source="worktree" onSourceChange={() => {}} onOpenFile={() => {}} />);
  await waitFor(() => expect(screen.getByRole('button', { name: 'fileWorkspace.nextHunk' })).toHaveProperty('disabled', false));
  fireEvent.click(screen.getByRole('button', { name: 'fileWorkspace.nextHunk' }));
  fireEvent.click(screen.getByRole('button', { name: 'fileWorkspace.undoHunk' }));
  await waitFor(() => expect(invoke).toHaveBeenCalledWith('execute_development_git', { input: { id: expect.any(String), task_id: 't', review_token: 'fresh-review', action: { kind: 'restore_hunk', path: 'a.ts', diff_hash: 'host-raw-diff-hash', hunk_index: 1, confirmed: true } } }));
  await act(async () => {}); vi.restoreAllMocks();
});

it('refuses undo when the displayed host diff hash has become stale', async () => {
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  render(<GitFileDiff projectId="p" taskId="t" workspacePath="/repo" path="a.ts" source="worktree" onSourceChange={() => {}} onOpenFile={() => {}} />);
  await waitFor(() => expect(screen.getByRole('button', { name: 'fileWorkspace.undoHunk' })).toHaveProperty('disabled', false));
  diff = { ...diff, diff_hash: 'newer-host-version' };
  fireEvent.click(screen.getByRole('button', { name: 'fileWorkspace.undoHunk' }));
  await screen.findByRole('alert');
  expect(invoke.mock.calls.some(([command]) => command === 'execute_development_git')).toBe(false);
});

it('keeps the editor locked after an ambiguous hunk mutation transport failure', async () => {
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  useDevelopment.setState({ load: vi.fn().mockResolvedValue(true) });
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === 'execute_development_git') throw 'git_unknown';
    return original(command, args);
  });
  render(<GitFileDiff projectId="p" taskId="t" workspacePath="/repo" path="a.ts" source="worktree" onSourceChange={() => {}} onOpenFile={() => {}} />);
  await waitFor(() => expect(screen.getByRole('button', { name: 'fileWorkspace.undoHunk' })).toHaveProperty('disabled', false));
  fireEvent.click(screen.getByRole('button', { name: 'fileWorkspace.undoHunk' }));
  await screen.findByRole('alert');
  const key = developmentDraftKey('delivery', 'p', 't', '/repo', 'git-hunk', 'a.ts');
  expect(useDevelopmentDrafts.getState().entries[key]?.value).toMatchObject({ operation: { status: 'unknown', requestId: expect.any(String), writeTask: 't' } });
});

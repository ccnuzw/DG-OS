import { useState } from 'react';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { GitCommitDetail } from '../src/components/development/GitCommitDetail';
import { useDevelopment } from '../src/stores/developmentStore';
import type { GitCommitDetails } from '../src/stores/developmentGitCommit';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
const oid = 'a'.repeat(40), parent = 'b'.repeat(40), secondParent = 'c'.repeat(40);
const details = (id = oid): GitCommitDetails => ({ oid: id, author: 'Alice', author_email: 'alice@example.test', timestamp: 1700000000, message: 'Real subject\n\nFull message body\nAnother paragraph', parents: [parent, secondParent], files: [{ path: 'src/new.ts', original_path: 'src/old.ts', status: 'R', added: 4, deleted: 2 }, { path: 'image.png', original_path: null, status: 'A', added: null, deleted: null }], truncated: false });
const props = { projectId: 'p', taskId: null, workspacePath: '/project', oid, onSelect: vi.fn(), onClose: vi.fn() };
beforeEach(() => { useDevelopment.getState().reset(); invoke.mockImplementation(async command => command === 'read_development_git_commit' ? details() : { path: 'src/new.ts', before: 'old', after: 'new', diff: '@@ -1 +1 @@\n-old\n+new', diff_hash: null, reason: null }); });
afterEach(() => { cleanup(); invoke.mockReset(); vi.restoreAllMocks(); useDevelopment.getState().reset(); });
it('shows full metadata, both parents and binary counts, then uses the exact immutable commit and renamed path', async () => {
  function Inspector() { const [selected, setSelected] = useState(oid); return <GitCommitDetail {...props} oid={selected} onSelect={setSelected} />; }
  invoke.mockImplementation(async (command, args) => command === 'read_development_git_commit' ? details(args.input.oid) : { path: args.input.path, before: 'old', after: 'new', diff: '@@ -1 +1 @@\n-old\n+new', diff_hash: null, reason: null });
  render(<Inspector />); await act(async () => {});
  expect(screen.getByText(/Full message body/)).toBeTruthy(); expect(screen.getByText('Alice <alice@example.test>')).toBeTruthy();
  expect(screen.getByText('gitPanel.unknownStats')).toBeTruthy(); expect(screen.getByText('gitPanel.firstParentComparison')).toBeTruthy();
  const parents = screen.getAllByRole('button', { name: 'gitPanel.inspectParent' }); expect(parents).toHaveLength(2);
  fireEvent.click(parents[1]); await act(async () => {});
  expect(invoke).toHaveBeenLastCalledWith('read_development_git_commit', { input: { project_id: 'p', task_id: null, oid: secondParent } });
  fireEvent.click(screen.getByRole('button', { name: /src\/old.ts → src\/new.ts/ })); await act(async () => {});
  expect(invoke).toHaveBeenLastCalledWith('read_development_git_commit_diff', { input: { project_id: 'p', task_id: null, oid: secondParent, path: 'src/new.ts' } });
  const diff = screen.getByTestId('dev-git-commit-diff'); expect(within(diff).getByText('-old')).toBeTruthy(); expect(within(diff).getByText('+new')).toBeTruthy();
  fireEvent.click(within(diff).getByRole('button', { name: 'review.before' })); expect(within(diff).getByRole('region', { name: 'review.before' }).textContent).toContain('old');
  fireEvent.click(within(diff).getByRole('button', { name: 'gitPanel.backToFiles' })); expect(screen.queryByTestId('dev-git-commit-diff')).toBeNull();
  expect(invoke.mock.calls.every(([command]) => command === 'read_development_git_commit' || command === 'read_development_git_commit_diff')).toBe(true);
});
it('discards late commit and file responses when selection or actual workspace changes', async () => {
  const oldCommit = Promise.withResolvers<GitCommitDetails>(), oldDiff = Promise.withResolvers<unknown>();
  invoke.mockImplementation((command, args) => command === 'read_development_git_commit' ? args.input.oid === oid ? oldCommit.promise : Promise.resolve(details(parent)) : oldDiff.promise);
  const view = render(<GitCommitDetail {...props} />);
  view.rerender(<GitCommitDetail {...props} oid={parent} />); await act(async () => {});
  await act(async () => { oldCommit.resolve({ ...details(), message: 'Obsolete commit' }); }); expect(screen.queryByText('Obsolete commit')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: /src\/old.ts → src\/new.ts/ })); await act(async () => {});
  view.rerender(<GitCommitDetail {...props} oid={parent} workspacePath="/other-worktree" />); await act(async () => {});
  await act(async () => { oldDiff.resolve({ path: 'src/new.ts', before: 'obsolete before', after: 'obsolete after', diff: 'obsolete diff', reason: null }); });
  expect(screen.queryByText('obsolete diff')).toBeNull(); expect(screen.queryByTestId('dev-git-commit-diff')).toBeNull();
});
it('discards a response after an epoch change', async () => {
  const pending = Promise.withResolvers<GitCommitDetails>(); invoke.mockReturnValueOnce(pending.promise);
  render(<GitCommitDetail {...props} />);
  await act(async () => { useDevelopment.setState(state => ({ epoch: state.epoch + 1 })); });
  await act(async () => { pending.resolve({ ...details(), message: 'Previous account commit' }); });
  expect(screen.queryByText('Previous account commit')).toBeNull();
});
it('retries commit and historical diff failures without changing the selected identity', async () => {
  invoke.mockRejectedValueOnce({ code: 'file_not_found' }); render(<GitCommitDetail {...props} />); await act(async () => {});
  expect(screen.getByRole('alert')).toBeTruthy(); fireEvent.click(screen.getByRole('button', { name: 'gitPanel.retry' })); await act(async () => {});
  invoke.mockRejectedValueOnce({ code: 'file_not_found' }); fireEvent.click(screen.getByRole('button', { name: /src\/old.ts → src\/new.ts/ })); await act(async () => {});
  expect(screen.getByRole('alert')).toBeTruthy(); fireEvent.click(screen.getByRole('button', { name: 'gitPanel.retry' })); await act(async () => {});
  expect(invoke).toHaveBeenLastCalledWith('read_development_git_commit_diff', { input: { project_id: 'p', task_id: null, oid, path: 'src/new.ts' } });
  expect(screen.getByText('+new')).toBeTruthy();
});
it('copies the complete hash and explicitly describes an empty root comparison', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined); Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  invoke.mockResolvedValue({ ...details(), parents: [], files: [], message: '' }); render(<GitCommitDetail {...props} />); await act(async () => {});
  expect(screen.getByText('gitPanel.rootCommit')).toBeTruthy(); expect(screen.getByText('gitPanel.emptyTreeComparison')).toBeTruthy(); expect(screen.getByText('gitPanel.noChangedFiles')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.copyHash' })); await act(async () => {}); expect(writeText).toHaveBeenCalledExactlyOnceWith(oid);
});

it.each(['file_binary', 'file_too_large', 'git_special_entry'])('keeps %s distinct from an absent added/deleted side', async reason => {
  invoke.mockImplementation(async command => command === 'read_development_git_commit' ? details() : { path: 'image.png', before: 'Excluded source', after: 'Excluded source', diff: 'Excluded patch', diff_hash: null, reason });
  render(<GitCommitDetail {...props} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: /image.png/ })); await act(async () => {});
  const diff = screen.getByTestId('dev-git-commit-diff');
  expect(within(diff).getByText(reason === 'git_special_entry' ? 'gitPanel.specialEntry' : `review.reason.${reason}`)).toBeTruthy();
  expect(within(diff).queryByRole('region', { name: 'review.unified' })).toBeNull(); expect(screen.queryByText('Excluded patch')).toBeNull();
  fireEvent.click(within(diff).getByRole('button', { name: 'review.before' }));
  expect(screen.queryByText('gitPanel.absentVersion')).toBeNull(); expect(within(diff).queryByRole('region', { name: 'review.before' })).toBeNull();
});

it('labels only an actually missing side as absent and keeps an empty text file readable', async () => {
  invoke.mockImplementation(async command => command === 'read_development_git_commit' ? details() : { path: 'src/new.ts', before: null, after: '', diff: '', diff_hash: null, reason: null });
  render(<GitCommitDetail {...props} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: /src\/old.ts → src\/new.ts/ })); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'review.before' })); expect(screen.getByText('gitPanel.absentVersion')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'review.after' })); expect(screen.getByRole('region', { name: 'review.after' })).toBeTruthy(); expect(screen.queryByText('gitPanel.absentVersion')).toBeNull();
});

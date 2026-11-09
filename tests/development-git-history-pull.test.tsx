import { StrictMode } from 'react';
import { Modal } from '@dgos/ui';
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { GitHistoryPull } from '../src/components/development/GitHistoryPull';
import type { GitReview } from '../src/stores/developmentDelivery';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
const review: GitReview = { task_id: 'task', kind: 'git', path: '/fixture', branch: 'main', head: 'a'.repeat(40), merge_heads: [], files: [], index: [], conflicts: [], staged_diff: '', worktree_diff: '', review_token: 'review', baseline_version: '', initial_staged_paths: [], file_versions: {} };
afterEach(() => { cleanup(); invoke.mockReset(); });
it('reads fixed-anchor history in StrictMode and renders untrusted subject as text', async () => {
  invoke.mockResolvedValueOnce({ anchor: review.head, commits: [{ oid: review.head, author: 'Alice', timestamp: 1, subject: '<img src=x onerror=alert(1)>', parents: [] }], next_cursor: 'cursor' });
  const view = render(<StrictMode><GitHistoryPull review={review} busy={false} blocked={false} act={vi.fn()} /></StrictMode>);
  fireEvent.click(screen.getByRole('button', { name: 'delivery.openGitHistory' })); await act(async () => {});
  expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeTruthy(); expect(view.container.querySelector('img')).toBeNull();
  invoke.mockResolvedValueOnce({ anchor: review.head, commits: [], next_cursor: null });
  fireEvent.click(screen.getByRole('button', { name: 'delivery.moreGitHistory' })); await act(async () => {});
  expect(invoke).toHaveBeenLastCalledWith('read_development_git_history', { input: { task_id: 'task', start_oid: review.head, cursor: 'cursor', limit: 30 } });
});
it('discards a slow preview when the remote selection changes', async () => {
  const deferred = Promise.withResolvers<unknown>(); invoke.mockReturnValue(deferred.promise);
  render(<GitHistoryPull review={review} busy={false} blocked={false} act={vi.fn()} />);
  fireEvent.click(screen.getByText('delivery.previewPull', { selector: 'summary' }));
  fireEvent.change(screen.getByLabelText('delivery.pullRemote'), { target: { value: 'origin' } });
  fireEvent.change(screen.getByLabelText('delivery.pullBranch'), { target: { value: 'main' } });
  fireEvent.click(screen.getByRole('button', { name: 'delivery.previewPull' }));
  fireEvent.change(screen.getByLabelText('delivery.pullRemote'), { target: { value: 'other' } });
  await act(async () => { deferred.resolve({ review_token: 'review', remote: 'origin', branch: 'main', remote_ref: 'refs/heads/main', remote_display: 'https://host/…', preview_token: 'old' }); });
  expect(screen.queryByRole('button', { name: 'delivery.pull' })).toBeNull();
});

it('invalidates a reviewed pull on native completion without fetching or pulling automatically', async () => {
 const preview = { review_token: review.review_token, remote: 'origin', branch: 'main', remote_ref: 'refs/heads/main', remote_display: '/local/fixture.git', preview_token: 'preview' };
 invoke.mockResolvedValue(preview); const actGit = vi.fn();
 const view = render(<GitHistoryPull review={review} busy={false} blocked={false} act={actGit} refreshKey={0} />);
 fireEvent.click(screen.getByText('delivery.previewPull', { selector: 'summary' }));
 fireEvent.change(screen.getByLabelText('delivery.pullRemote'), { target: { value: 'origin' } });
 fireEvent.change(screen.getByLabelText('delivery.pullBranch'), { target: { value: 'main' } });
 fireEvent.click(screen.getByRole('button', { name: 'delivery.previewPull' })); await act(async () => {});
 expect(screen.getByRole('button', { name: 'delivery.pull' })).toBeTruthy();
 view.rerender(<GitHistoryPull review={review} busy={false} blocked={false} act={actGit} refreshKey={1} />); await act(async () => {});
 expect(screen.queryByRole('button', { name: 'delivery.pull' })).toBeNull();
 expect((screen.getByLabelText('delivery.pullRemote') as HTMLInputElement).value).toBe('origin');
 expect((screen.getByLabelText('delivery.pullBranch') as HTMLInputElement).value).toBe('main');
 expect(invoke).toHaveBeenCalledTimes(1); expect(actGit).not.toHaveBeenCalled();
});

it('requires explicit pull confirmation and forwards the exact preview token and trust', async () => {
 const preview = { review_token: review.review_token, remote: 'origin', branch: 'main', remote_ref: 'refs/heads/main', remote_display: '/local/fixture.git', preview_token: 'preview' };
 invoke.mockResolvedValue(preview); const actGit = vi.fn().mockResolvedValue(undefined);
 const confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
 try {
  render(<GitHistoryPull review={review} busy={false} blocked={false} act={actGit} />);
  fireEvent.click(screen.getByText('delivery.previewPull', { selector: 'summary' }));
  fireEvent.change(screen.getByLabelText('delivery.pullRemote'), { target: { value: 'origin' } });
  fireEvent.change(screen.getByLabelText('delivery.pullBranch'), { target: { value: 'main' } });
  fireEvent.click(screen.getByRole('button', { name: 'delivery.previewPull' })); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'delivery.pull' })); await act(async () => {});
  expect(confirm.mock.calls[0][0].content).toContain(preview.remote_display); expect(actGit).not.toHaveBeenCalled();
  confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  fireEvent.click(screen.getByRole('button', { name: 'delivery.pull' })); await act(async () => {});
  expect(actGit).toHaveBeenCalledExactlyOnceWith({ kind: 'pull', remote: 'origin', branch: 'main', preview_token: 'preview', trust_directory: true, trust_network: true });
 } finally { confirm.mockRestore(); }
});

it('drops old history on a new HEAD without automatically reloading history', async () => {
 invoke.mockResolvedValue({ anchor: review.head, commits: [{ oid: review.head, author: 'Person', timestamp: 1, subject: 'Old anchored entry', parents: [] }], next_cursor: null });
 const actGit = vi.fn(); const view = render(<GitHistoryPull review={review} busy={false} blocked={false} act={actGit} />);
 fireEvent.click(screen.getByRole('button', { name: 'delivery.openGitHistory' })); await act(async () => {});
 expect(screen.getByText('Old anchored entry')).toBeTruthy();
 view.rerender(<GitHistoryPull review={{ ...review, head: 'b'.repeat(40), review_token: 'new' }} busy={false} blocked={false} act={actGit} />); await act(async () => {});
 expect(screen.queryByText('Old anchored entry')).toBeNull(); expect(invoke).toHaveBeenCalledTimes(1); expect(actGit).not.toHaveBeenCalled();
});

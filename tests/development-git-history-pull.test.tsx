import { StrictMode } from 'react';
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
  fireEvent.change(screen.getByLabelText('delivery.pullRemote'), { target: { value: 'origin' } });
  fireEvent.change(screen.getByLabelText('delivery.pullBranch'), { target: { value: 'main' } });
  fireEvent.click(screen.getByRole('button', { name: 'delivery.previewPull' }));
  fireEvent.change(screen.getByLabelText('delivery.pullRemote'), { target: { value: 'other' } });
  await act(async () => { deferred.resolve({ review_token: 'review', remote: 'origin', branch: 'main', remote_ref: 'refs/heads/main', remote_display: 'https://host/…', preview_token: 'old' }); });
  expect(screen.queryByRole('button', { name: 'delivery.pull' })).toBeNull();
});

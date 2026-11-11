import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { Modal, ModalHost } from '@dgos/ui';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { GitPanel } from '../src/components/development/GitPanel';
import { useCommitMessageGenerator } from '../src/components/development/CommitMessageGenerator';
import { useDevelopment } from '../src/stores/developmentStore';
import { developmentDraftKey, useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { developmentEditorMutationAffects, documentId, useDevelopmentEditor } from '../src/stores/developmentEditor';
import { chooseContext } from '../src/stores/developmentSelection';
import type { ExecuteGitInput, GitReview } from '../src/stores/developmentDelivery';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
const project = { id: 'p', owner: 'alice', name: 'Project', path: '/workspace', directory_identity: 'dir', archived: false, created_at: 1, last_opened_at: null };
const props = { projectId: 'p', taskId: 't', workspacePath: '/workspace', sessions: [], live: {} };
const base = (): GitReview => ({ task_id: 't', kind: 'git', path: '/workspace', branch: 'main', head: 'a'.repeat(40), files: [{ path: 'src/file.ts', index: 'M', worktree: 'M' }], index: [], conflicts: [], snapshot_token: 'snapshot', review_token: 'review', initial_staged_paths: [], merge_heads: [], baseline_version: '', file_versions: {} });
let review: GitReview;
let receipts: unknown[];
beforeEach(() => {
  localStorage.clear(); useDevelopment.getState().reset(); useDevelopment.setState({ projects: [project] }); review = base(); receipts = [];
  invoke.mockImplementation(async (command: string, args: { input: ExecuteGitInput }) => {
    if (command === 'read_development_project_git') return { kind: review.kind, path: review.path, branch: review.branch, head: review.head, merge_heads: review.merge_heads, files: review.files, conflicts: review.conflicts, snapshot_token: review.snapshot_token };
    if (command === 'read_development_git') return review;
    if (command === 'read_development_staged_diff') return { snapshot_token: review.snapshot_token, diff: 'staged changes' };
    if (command === 'read_development_git_details') return { branches: ['main', 'topic'], remotes: ['origin'], remote_branches: { origin: ['main', 'topic'] }, remote_push_targets: { origin: ['https://example.test/push.git'] }, remote_fetch_targets: { origin: ['https://example.test/fetch.git'] }, upstream: 'origin/main', ahead: 1, behind: 0, files: [{ path: 'src/file.ts', index_added: 2, index_deleted: 1, worktree_added: 3, worktree_deleted: 0 }], history: [], history_truncated: false };
    if (command === 'read_development_workspaces') return { workspaces: [], selected_workspace_id: null, operation: null, operations: receipts };
    if (command === 'execute_development_git') { const receipt = { id: args.input.id, kind: `git_${args.input.action.kind}`, status: 'succeeded', project_id: 'p', task_id: 't', workspace_id: null, request_fingerprint: '', details: {} }; receipts = [receipt]; return receipt; }
    if (command === 'read_development_workspace') return { projects: [project], tasks: [], sessions: [], operations: receipts, selection: { project_id: 'p', task_id: 't', session_id: null } };
    throw new Error(`Unexpected command ${command}`);
  });
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); invoke.mockReset(); useDevelopment.getState().reset(); useDevelopmentDrafts.getState().clear(); localStorage.clear(); });
const mutations = () => invoke.mock.calls.filter(([command]) => ['execute_development_git', 'preview_development_git_pull', 'mutate_development_entry'].includes(command));
it('opens diffs, refreshes, and browses remote choices without sending, staging, or creating a task', async () => {
  const ensureTask = vi.fn(), onOpenDiff = vi.fn(), generate = vi.fn();
  const view = render(<GitPanel {...props} taskId={null} ensureTask={ensureTask} onOpenDiff={onOpenDiff} onGenerateMessage={generate} />);
  await act(async () => {});
  fireEvent.click(screen.getAllByRole('button', { name: 'file.ts' })[0]);
  expect(onOpenDiff).toHaveBeenCalledWith('src/file.ts', 'index');
  fireEvent.click(screen.getByRole('button', { name: 'delivery.pull' }));
  fireEvent.change(screen.getByLabelText('delivery.remote'), { target: { value: 'origin' } });
  fireEvent.change(screen.getByLabelText('delivery.branch'), { target: { value: 'topic' } });
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.previewTarget' }));
  await act(async () => {});
  expect(ensureTask).not.toHaveBeenCalled(); expect(generate).not.toHaveBeenCalled(); expect(mutations()).toEqual([]);
  view.unmount();
});
it('rejects stale stage actions after reloading the Git review', async () => {
  render(<GitPanel {...props} />); await act(async () => {});
  review = { ...review, branch: 'topic', snapshot_token: 'changed-snapshot', review_token: 'changed' };
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.stageAll' })); await act(async () => {});
  expect(mutations()).toEqual([]); expect(screen.getByRole('alert')).toBeTruthy();
});
it('commits only after explicit full-index approval without silently staging files', async () => {
  const confirmation = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close() {} }; });
  render(<GitPanel {...props} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Reviewed commit' } });
  fireEvent.click(screen.getByRole('button', { name: 'delivery.commit' })); await act(async () => {});
  expect(mutations()).toEqual([]); expect(confirmation.mock.calls[0][0].content).toContain('src/file.ts');
  confirmation.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close() {} }; });
  fireEvent.click(screen.getByRole('button', { name: 'delivery.commit' })); await act(async () => {});
  expect(mutations()).toHaveLength(1);
  expect(mutations()[0][1].input.action).toEqual({ kind: 'commit', message: 'Reviewed commit', confirm_full_index: true, confirm_preexisting_staged: true });
});
it('protects edits made during generation and cancels the actual request', async () => {
  const pending = Promise.withResolvers<string | null>();
  const generate = vi.fn((_diff: string, _signal: AbortSignal) => pending.promise);
  render(<GitPanel {...props} taskId={null} onGenerateMessage={generate} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.generate' }));
  await act(async () => {});
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'My own message' } });
  await act(async () => { pending.resolve('Generated message'); });
  expect((screen.getByLabelText('delivery.commitMessage') as HTMLTextAreaElement).value).toBe('My own message');
  const cancelled = Promise.withResolvers<string | null>(); generate.mockImplementation(() => cancelled.promise);
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.generate' }));
  await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.cancelGeneration' }));
  expect(generate.mock.calls[1][1].aborted).toBe(true);
  await act(async () => { cancelled.resolve('Cancelled message'); });
  expect((screen.getByLabelText('delivery.commitMessage') as HTMLTextAreaElement).value).toBe('My own message');
  expect(mutations()).toEqual([]);
});
it('keeps durable unknown receipts blocked after reopening without resending', async () => {
  localStorage.setItem('dgos.delivery.requests:alice', JSON.stringify([{ id: 'unknown-request', project_id: 'p', task_id: 't', kind: 'git_commit' }]));
  render(<GitPanel {...props} />); await act(async () => {});
  expect((screen.getByRole('button', { name: 'gitPanel.stageAll' }) as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByRole('button', { name: /delivery.reconcile/ })).toBeTruthy(); expect(mutations()).toEqual([]);
});
it('does not push or offer a network confirmation after an unknown commit result', async () => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: { input: ExecuteGitInput }) => {
    if (command === 'execute_development_git') {
      const receipt = { id: args.input.id, kind: 'git_commit', status: 'unknown', project_id: 'p', task_id: 't', workspace_id: null, request_fingerprint: '', details: {} };
      receipts = [receipt]; return receipt;
    }
    return original(command, args);
  });
  vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close() {} }; });
  render(<GitPanel {...props} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Commit then push' } });
  fireEvent.keyDown(screen.getByRole('button', { name: 'gitPanel.commitActions' }), { key: 'ArrowDown' }); await act(async () => {});
  fireEvent.click(screen.getByRole('menuitem', { name: 'gitPanel.commitPush' })); await act(async () => {});
  expect(mutations()).toHaveLength(1); expect(mutations()[0][1].input.action.kind).toBe('commit');
  expect(screen.queryByRole('dialog', { name: 'delivery.push' })).toBeNull();
  expect((screen.getByLabelText('delivery.commitMessage') as HTMLTextAreaElement).value).toBe('Commit then push');
});
it('locks affected documents while a branch switch is unresolved', async () => {
  const original = invoke.getMockImplementation()!;
  const pending = Promise.withResolvers<unknown>();
  invoke.mockImplementation(async (command: string, args: { input: ExecuteGitInput }) => command === 'execute_development_git' ? pending.promise : original(command, args));
  render(<GitPanel {...props} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('gitPanel.branch'), { target: { value: 'topic' } }); await act(async () => {});
  expect(developmentEditorMutationAffects('p', 't', 'src/file.ts')).toBe(true);
  const call = mutations()[0][1].input;
  await act(async () => { const receipt = { id: call.id, kind: 'git_switch', status: 'unknown', project_id: 'p', task_id: 't', workspace_id: null, request_fingerprint: '', details: {} }; receipts = [receipt]; pending.resolve(receipt); });
  expect(developmentEditorMutationAffects('p', 't', 'src/file.ts')).toBe(true);
  expect((screen.getByRole('button', { name: 'gitPanel.stageAll' }) as HTMLButtonElement).disabled).toBe(true);
});

it('renders compact selectable history with full-date titles and forwards full immutable ids without task creation', async () => {
  const original = invoke.getMockImplementation()!, oid = 'f'.repeat(40), onSelectCommit = vi.fn(), ensureTask = vi.fn();
  invoke.mockImplementation(async (command: string, args: unknown) => {
    const result = await original(command, args);
    return command === 'read_development_git_details' ? { ...result, history: [{ oid, subject: 'Full history subject', author: 'Alice', timestamp: 1700000000, parents: [] }], history_truncated: true } : result;
  });
  render(<GitPanel {...props} taskId={null} ensureTask={ensureTask} selectedCommit={oid} onSelectCommit={onSelectCommit} />); await act(async () => {});
  const row = screen.getByRole('button', { name: /Full history subject/ });
  expect(row.getAttribute('aria-pressed')).toBe('true'); expect(row.getAttribute('data-commit-oid')).toBe(oid); expect(row.title).toContain('Alice'); expect(row.title).toContain('2023');
  expect(row.firstElementChild?.textContent).toBe('fffffff'); expect(row.lastElementChild?.tagName).toBe('TIME'); expect(screen.getByRole('button', { name: 'delivery.openGitHistory' })).toBeTruthy();
  fireEvent.click(row); expect(onSelectCommit).toHaveBeenCalledExactlyOnceWith(oid, row, '/workspace'); expect(ensureTask).not.toHaveBeenCalled(); expect(mutations()).toEqual([]);
});

it('opens a path-history commit in the adjacent inspector and closes the history picker', async () => {
  const original = invoke.getMockImplementation()!, oid = 'e'.repeat(40), onSelectCommit = vi.fn();
  invoke.mockImplementation(async (command: string, args: { input?: { path?: string } }) => {
    const result = await original(command, args);
    return command === 'read_development_git_details' && args.input?.path ? { ...result, history: [{ oid, subject: 'Path-specific commit', author: 'Alice', timestamp: 1700000000, parents: [] }] } : result;
  });
  render(<GitPanel {...props} onSelectCommit={onSelectCommit} />); await act(async () => {});
  fireEvent.keyDown(screen.getAllByRole('button', { name: 'gitPanel.fileActions' })[0], { key: 'ArrowDown' }); await act(async () => {});
  fireEvent.click(screen.getByRole('menuitem', { name: 'gitPanel.pathHistory' })); await act(async () => {});
  const picker = screen.getByRole('dialog', { name: 'gitPanel.pathHistory' });
  fireEvent.click(within(picker).getByRole('button', { name: /Path-specific commit/ })); await act(async () => {});
  expect(onSelectCommit.mock.calls[0][0]).toBe(oid); expect(onSelectCommit.mock.calls[0][2]).toBe('/workspace'); expect(screen.queryByRole('dialog', { name: 'gitPanel.pathHistory' })).toBeNull(); expect(mutations()).toEqual([]);
});

it('rejects a changed raw snapshot even when file statuses and visible content are identical', async () => {
  render(<GitPanel {...props} />); await act(async () => {});
  review = { ...review, snapshot_token: 'different-raw-bytes', review_token: 'different-authority', file_versions: { 'src/file.ts': 'changed-hash' } };
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.stageAll' })); await act(async () => {});
  expect(mutations()).toEqual([]);
  expect(screen.getByRole('alert')).toBeTruthy();
});

it('invalidates an old commit disclosure when project props change during provider loading', async () => {
  const provider = Promise.withResolvers<unknown>();
  invoke.mockReturnValue(provider.promise);
  const completed = vi.fn();
  function Generator({ projectId }: { projectId: string }) {
    const { generate, dialog } = useCommitMessageGenerator(projectId, null);
    return <><button onClick={() => void generate('staged changes', new AbortController().signal).then(completed)}>start draft</button>{dialog}</>;
  }
  const view = render(<Generator projectId="old-project" />);
  fireEvent.click(screen.getByRole('button', { name: 'start draft' }));
  view.rerender(<Generator projectId="new-project" />);
  await act(async () => { provider.resolve({ revision: 1, configuration: { provider: { provider_id: 'old-provider', model_id: 'old-model' } } }); });
  expect(completed).toHaveBeenCalledWith(null);
  expect(screen.queryByRole('dialog', { name: 'commitDraft.title' })).toBeNull();
  expect(invoke.mock.calls.some(([command]) => command === 'generate_development_commit_message')).toBe(false);
});

it('browses task Git through its scoped observation without strict reviews or staged previews', async () => {
  render(<GitPanel {...props} />); await act(async () => {});
  expect(invoke).toHaveBeenCalledWith('read_development_project_git', { projectId: 'p', taskId: 't' });
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_git' || command === 'read_development_staged_diff')).toBe(false);
  expect(screen.getAllByRole('button', { name: 'file.ts' })).toHaveLength(2);
});

it('uses a newly fetched strict token only after matching the displayed snapshot', async () => {
  render(<GitPanel {...props} />); await act(async () => {});
  review = { ...review, review_token: 'new-strict-authority' };
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.stageAll' })); await act(async () => {});
  expect(mutations()).toHaveLength(1);
  expect(mutations()[0][1].input.review_token).toBe('new-strict-authority');
});

it('keeps the visible list and commit draft when a strict write review is incomplete', async () => {
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: unknown) => { if (command === 'read_development_git') throw 'git_review_incomplete'; return original(command, args); });
  render(<GitPanel {...props} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Unsent local draft' } });
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.stageAll' })); await act(async () => {});
  expect(screen.getByRole('alert').textContent).toBe('dev.error.git_review_incomplete');
  expect(screen.getAllByRole('button', { name: 'file.ts' })).toHaveLength(2);
  expect(screen.getByLabelText('delivery.commitMessage')).toHaveProperty('value', 'Unsent local draft');
  expect(mutations()).toEqual([]);
});

it('loads only the staged preview on an explicit generation click', async () => {
  const generate = vi.fn().mockResolvedValue('Generated from staged changes');
  render(<GitPanel {...props} onGenerateMessage={generate} />); await act(async () => {});
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_staged_diff')).toBe(false);
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.generate' })); await act(async () => {});
  expect(invoke).toHaveBeenCalledWith('read_development_staged_diff', { input: { project_id: 'p', task_id: 't' } });
  expect(generate).toHaveBeenCalledExactlyOnceWith('staged changes', expect.any(AbortSignal));
  expect(screen.getByLabelText('delivery.commitMessage')).toHaveProperty('value', 'Generated from staged changes');
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_git')).toBe(false);
});

it.each(['git_output_limit', 'git_review_incomplete'])('keeps %s local to generation without clearing the draft or list', async failure => {
  const original = invoke.getMockImplementation()!, generate = vi.fn();
  invoke.mockImplementation(async (command: string, args: unknown) => { if (command === 'read_development_staged_diff') throw failure; return original(command, args); });
  render(<GitPanel {...props} onGenerateMessage={generate} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Keep this unsent draft' } });
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.generate' })); await act(async () => {});
  expect(screen.getByRole('alert').textContent).toBe(`dev.error.${failure}`);
  expect(screen.getByLabelText('delivery.commitMessage')).toHaveProperty('value', 'Keep this unsent draft');
  expect(screen.getAllByRole('button', { name: 'file.ts' })).toHaveLength(2);
  expect(screen.getByRole('button', { name: 'gitPanel.stageAll' })).not.toHaveProperty('disabled', true);
  expect(generate).not.toHaveBeenCalled(); expect(mutations()).toEqual([]);
});

it('refuses to send a staged preview from a different snapshot', async () => {
  const original = invoke.getMockImplementation()!, generate = vi.fn();
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'read_development_staged_diff' ? { snapshot_token: 'changed-raw-index', diff: 'new staged bytes' } : original(command, args));
  render(<GitPanel {...props} onGenerateMessage={generate} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.generate' })); await act(async () => {});
  expect(screen.getByRole('alert').textContent).toBe('dev.error.git_review_stale');
  expect(generate).not.toHaveBeenCalled(); expect(mutations()).toEqual([]);
});

it.each(['cancel', 'context', 'workspace'])('never calls the generator after %s invalidates a pending preview', async invalidation => {
  const original = invoke.getMockImplementation()!, generate = vi.fn(), preview = Promise.withResolvers<unknown>();
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'read_development_staged_diff' ? preview.promise : command === 'select_development_context' ? null : original(command, args));
  const view = render(<GitPanel {...props} onGenerateMessage={generate} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Draft survives a late preview' } });
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.generate' }));
  if (invalidation === 'cancel') fireEvent.click(screen.getByRole('button', { name: 'gitPanel.cancelGeneration' }));
  else if (invalidation === 'context') await act(async () => { await chooseContext({ project_id: 'other-project', task_id: null, session_id: null }); });
  else { view.rerender(<GitPanel {...props} workspacePath="/different-workspace" onGenerateMessage={generate} />); await act(async () => {}); }
  await act(async () => { preview.resolve({ snapshot_token: 'snapshot', diff: 'late staged patch' }); });
  expect(generate).not.toHaveBeenCalled();
  expect(invoke.mock.calls.some(([command]) => command === 'generate_development_commit_message')).toBe(false);
  if (invalidation !== 'workspace') expect(screen.getByLabelText('delivery.commitMessage')).toHaveProperty('value', 'Draft survives a late preview');
});

it('preserves merge-only commit enablement without inventing a staged patch', async () => {
  review = { ...review, files: [], merge_heads: ['merge-head'] };
  render(<GitPanel {...props} onGenerateMessage={vi.fn()} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Finish merge' } });
  expect(screen.getByRole('button', { name: 'delivery.commit' })).not.toHaveProperty('disabled', true);
  expect(screen.getByRole('button', { name: 'gitPanel.generate' })).toHaveProperty('disabled', true);
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_staged_diff')).toBe(false);
});

it('keeps provider failures local after a successful staged preview', async () => {
  const generate = vi.fn().mockRejectedValue(Error('Provider generation failed'));
  render(<GitPanel {...props} onGenerateMessage={generate} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'Existing commit message' } });
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.generate' })); await act(async () => {});
  expect(screen.getByRole('alert').textContent).toBe('Provider generation failed');
  expect(screen.getByLabelText('delivery.commitMessage')).toHaveProperty('value', 'Existing commit message');
  expect(screen.getAllByRole('button', { name: 'file.ts' })).toHaveLength(2);
  expect(mutations()).toEqual([]);
});

it.each([null, 't'])('paginates %s history against the fixed original anchor without obtaining write authority', async taskId => {
  const original = invoke.getMockImplementation()!, older = 'b'.repeat(40), ensureTask = vi.fn(), select = vi.fn();
  invoke.mockImplementation(async (command: string, args: { input?: { cursor?: string | null } }) => {
    if (command === 'read_development_git_history' || command === 'read_development_project_git_history') return { anchor: review.head, commits: [{ oid: args.input?.cursor ? older : review.head, author: 'Alice', timestamp: 1, subject: args.input?.cursor ? 'Older paginated commit' : '<img src=x onerror=alert(1)>', parents: [] }], next_cursor: args.input?.cursor ? null : 'opaque-cursor' };
    const result = await original(command, args);
    return command === 'read_development_git_details' ? { ...result, history_truncated: true } : result;
  });
  const view = render(<GitPanel {...props} taskId={taskId} ensureTask={ensureTask} onSelectCommit={select} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'delivery.openGitHistory' })); await act(async () => {});
  expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeTruthy(); expect(view.container.querySelector('img')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'delivery.moreGitHistory' })); await act(async () => {});
  expect(screen.getByText('Older paginated commit')).toBeTruthy(); expect(screen.getByText('<img src=x onerror=alert(1)>')).toBeTruthy();
  expect(invoke).toHaveBeenLastCalledWith(taskId ? 'read_development_git_history' : 'read_development_project_git_history', { input: { ...(taskId ? { task_id: taskId } : { project_id: 'p' }), start_oid: review.head, cursor: 'opaque-cursor', limit: 50 } });
  fireEvent.click(screen.getByRole('button', { name: /Older paginated commit/ })); expect(select.mock.calls[0][0]).toBe(older);
  expect(screen.queryByRole('button', { name: 'delivery.moreGitHistory' })).toBeNull(); expect(ensureTask).not.toHaveBeenCalled(); expect(mutations()).toEqual([]);
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_git')).toBe(false);
});

it('drops pending history pages after a refreshed HEAD and never reloads pages automatically', async () => {
  const original = invoke.getMockImplementation()!, pending = Promise.withResolvers<unknown>();
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === 'read_development_git_history') return pending.promise;
    const result = await original(command, args);
    return command === 'read_development_git_details' ? { ...result, history_truncated: true } : result;
  });
  const view = render(<GitPanel {...props} refreshKey={0} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'delivery.openGitHistory' })); await act(async () => {});
  review = { ...review, head: 'b'.repeat(40) }; view.rerender(<GitPanel {...props} refreshKey={1} />); await act(async () => {});
  await act(async () => { pending.resolve({ anchor: 'a'.repeat(40), next_cursor: 'old', commits: [{ oid: 'a'.repeat(40), author: 'Alice', timestamp: 1, subject: 'Obsolete page', parents: [] }] }); });
  expect(screen.queryByText('Obsolete page')).toBeNull(); expect(invoke.mock.calls.filter(([command]) => command === 'read_development_git_history')).toHaveLength(1); expect(mutations()).toEqual([]);
});

it('keeps already loaded history when the next page fails and retries the same cursor', async () => {
  const original = invoke.getMockImplementation()!; let attempts = 0;
  invoke.mockImplementation(async (command: string, args: { input?: { cursor?: string | null } }) => {
    if (command === 'read_development_git_history') {
      if (!args.input?.cursor) return { anchor: review.head, next_cursor: 'retained-cursor', commits: [{ oid: review.head, author: 'Alice', timestamp: 1, subject: 'Retained first page', parents: [] }] };
      if (++attempts === 1) throw 'git_output_limit';
      return { anchor: review.head, next_cursor: null, commits: [] };
    }
    const result = await original(command, args); return command === 'read_development_git_details' ? { ...result, history_truncated: true } : result;
  });
  render(<GitPanel {...props} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'delivery.openGitHistory' })); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'delivery.moreGitHistory' })); await act(async () => {});
  expect(screen.getByRole('alert').textContent).toBe('dev.error.git_output_limit'); expect(screen.getByText('Retained first page')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'delivery.moreGitHistory' })); await act(async () => {});
  expect(invoke).toHaveBeenLastCalledWith('read_development_git_history', { input: { task_id: 't', start_oid: review.head, cursor: 'retained-cursor', limit: 50 } }); expect(mutations()).toEqual([]);
});

it.each(['empty', 'same', 'different'])('migrates an old workspace commit draft against a %s unified draft without overwriting conflicts', async current => {
  const original = invoke.getMockImplementation()!, oldKey = developmentDraftKey('commit', 'p', 't', 'legacy-workspace'), newKey = developmentDraftKey('commit', 'p', 't', '/workspace'), otherKey = developmentDraftKey('commit', 'p', 'other-task', 'legacy-workspace');
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'read_development_workspaces' ? { workspaces: [{ id: 'legacy-workspace', kind: 'managed', path: '/workspace' }], selected_workspace_id: 'legacy-workspace', operations: [], operation: null } : original(command, args));
  useDevelopmentDrafts.setState({ entries: {
    [oldKey]: { kind: 'commit', value: 'Legacy unsent draft', baseline: '' },
    [otherKey]: { kind: 'commit', value: 'Private other task draft', baseline: '' },
    ...(current === 'empty' ? {} : { [newKey]: { kind: 'commit' as const, value: current === 'same' ? 'Legacy unsent draft' : 'Current unsent draft', baseline: '' } }),
  } });
  render(<GitPanel {...props} />); await act(async () => {});
  expect(screen.getByLabelText('delivery.commitMessage')).toHaveProperty('value', current === 'different' ? 'Current unsent draft' : 'Legacy unsent draft');
  if (current === 'different') { expect(screen.getByLabelText('gitPanel.savedDraft')).toHaveProperty('value', 'Legacy unsent draft'); expect(useDevelopmentDrafts.getState().entries[oldKey].value).toBe('Legacy unsent draft'); }
  else expect(useDevelopmentDrafts.getState().entries[oldKey]).toBeUndefined();
  expect(useDevelopmentDrafts.getState().entries[otherKey].value).toBe('Private other task draft'); expect(screen.queryByDisplayValue('Private other task draft')).toBeNull(); expect(mutations()).toEqual([]);
});

it('does not assign a legacy workspace draft to a different selected directory', async () => {
  const original = invoke.getMockImplementation()!, oldKey = developmentDraftKey('commit', 'p', 't', 'other-workspace');
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'read_development_workspaces' ? { workspaces: [{ id: 'other-workspace', kind: 'managed', path: '/other' }], selected_workspace_id: 'other-workspace', operations: [], operation: null } : original(command, args));
  useDevelopmentDrafts.setState({ entries: { [oldKey]: { kind: 'commit', value: 'Other workspace unsent draft', baseline: '' } } });
  render(<GitPanel {...props} />); await act(async () => {});
  expect(screen.getByLabelText('delivery.commitMessage')).toHaveProperty('value', ''); expect(screen.queryByDisplayValue('Other workspace unsent draft')).toBeNull(); expect(useDevelopmentDrafts.getState().entries[oldKey].value).toBe('Other workspace unsent draft');
});

it('keeps pull preview read-only, requires explicit trust and forwards only the fresh exact preview token', async () => {
  review = { ...review, files: [] };
  const original = invoke.getMockImplementation()!, confirm = vi.spyOn(Modal, 'confirm').mockImplementation(options => { options.onCancel?.(); return { id: 'confirm', close: () => {} }; });
  invoke.mockImplementation(async (command: string, args: unknown) => command === 'preview_development_git_pull' ? { review_token: review.review_token, remote: 'origin', branch: 'main', remote_display: 'https://example.test/fetch.git', remote_ref: 'refs/heads/main', preview_token: 'fresh-preview' } : original(command, args));
  render(<GitPanel {...props} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'delivery.pull' }));
  const dialog = screen.getByRole('dialog', { name: 'delivery.pull' });
  fireEvent.change(within(dialog).getByLabelText('delivery.remote'), { target: { value: 'origin' } });
  fireEvent.change(within(dialog).getByLabelText('delivery.branch'), { target: { value: 'main' } });
  fireEvent.click(within(dialog).getByRole('button', { name: 'gitPanel.previewTarget' })); await act(async () => {});
  expect(mutations()).toEqual([]);
  fireEvent.click(within(dialog).getByRole('button', { name: 'delivery.pull' })); await act(async () => {});
  expect(confirm.mock.calls[0][0].content).toContain('https://example.test/fetch.git'); expect(mutations()).toEqual([]);
  confirm.mockImplementation(options => { options.onConfirm?.(); return { id: 'confirm', close: () => {} }; });
  fireEvent.click(within(dialog).getByRole('button', { name: 'delivery.pull' })); await act(async () => {});
  const write = invoke.mock.calls.find(([command]) => command === 'execute_development_git')!;
  expect(write[1].input.action).toEqual({ kind: 'pull', remote: 'origin', branch: 'main', preview_token: 'fresh-preview', trust_directory: true, trust_network: true });
});

it('keeps merge as one guarded Git action and locks documents for an unknown merge receipt', async () => {
  review = { ...review, files: [] };
  const original = invoke.getMockImplementation()!;
  invoke.mockImplementation(async (command: string, args: { input: ExecuteGitInput }) => {
    if (command === 'execute_development_git') {
      const result = { id: args.input.id, kind: 'git_merge', status: 'unknown', project_id: 'p', task_id: 't', workspace_id: null, request_fingerprint: '', details: {} };
      receipts = [result]; return result;
    }
    return original(command, args);
  });
  render(<GitPanel {...props} />); await act(async () => {});
  fireEvent.keyDown(screen.getByRole('button', { name: 'gitPanel.mergeBranches' }), { key: 'ArrowDown' }); await act(async () => {});
  fireEvent.click(screen.getByRole('menuitem', { name: 'gitPanel.mergeBranch' })); await act(async () => {});
  const writes = invoke.mock.calls.filter(([command]) => command === 'execute_development_git');
  expect(writes).toHaveLength(1); expect(writes[0][1].input.action).toEqual({ kind: 'merge', branch: 'topic' });
  expect(developmentEditorMutationAffects('p', 't', 'any-file.ts')).toBe(true);
  expect(screen.getByRole('button', { name: 'gitPanel.mergeBranches' })).toHaveProperty('disabled', true);
});

it('invalidates a reviewed pull target on native refresh without pulling or fetching automatically', async () => {
  const view = render(<GitPanel {...props} refreshKey={0} />); await act(async () => {});
  fireEvent.click(screen.getByRole('button', { name: 'delivery.pull' }));
  const dialog = screen.getByRole('dialog', { name: 'delivery.pull' });
  fireEvent.change(within(dialog).getByLabelText('delivery.remote'), { target: { value: 'origin' } });
  fireEvent.change(within(dialog).getByLabelText('delivery.branch'), { target: { value: 'main' } });
  fireEvent.click(within(dialog).getByRole('button', { name: 'gitPanel.previewTarget' })); await act(async () => {});
  expect(within(dialog).getByText(/https:\/\/example\.test\/fetch\.git/)).toBeTruthy();
  view.rerender(<GitPanel {...props} refreshKey={1} />); await act(async () => {});
  expect(within(dialog).queryByText(/https:\/\/example\.test\/fetch\.git/)).toBeNull();
  expect(within(dialog).getByLabelText('delivery.remote')).toHaveProperty('value', 'origin'); expect(mutations()).toEqual([]);
});

it.each(['restore', 'switch', 'pull', 'merge'] as const)('reviews again after saving before %s and requires a new gesture for changed snapshots', async kind => {
  if (kind === 'pull' || kind === 'merge') review = { ...review, files: [] };
  const original = invoke.getMockImplementation()!, saved = Promise.withResolvers<unknown>();
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === 'read_development_file') return { path: 'src/file.ts', text: 'base', hash: 'baseline', size: 4, reason: null };
    if (command === 'save_development_file') return saved.promise;
    return original(command, args);
  });
  await useDevelopmentEditor.getState().open('p', 't', 'src/file.ts', '/workspace');
  useDevelopmentEditor.getState().edit('approved saved draft');
  const showConfirm = Modal.confirm.bind(Modal);
  const confirmation = vi.spyOn(Modal, 'confirm').mockImplementation(options => {
    if (options.title === 'workbenchFiles.unsavedTitle') return showConfirm(options);
    options.onConfirm?.(); return { id: 'confirmed', close() {} };
  });
  const changed = vi.fn();
  render(<><GitPanel {...props} onChanged={changed} /><ModalHost /></>); await act(async () => {});
  if (kind === 'restore') fireEvent.click(screen.getByRole('button', { name: 'gitPanel.restore' }));
  else if (kind === 'switch') fireEvent.change(screen.getByLabelText('gitPanel.branch'), { target: { value: 'topic' } });
  else if (kind === 'merge') {
    fireEvent.keyDown(screen.getByRole('button', { name: 'gitPanel.mergeBranches' }), { key: 'ArrowDown' }); await act(async () => {});
    fireEvent.click(screen.getByRole('menuitem', { name: 'gitPanel.mergeBranch' }));
  } else {
    fireEvent.click(screen.getByRole('button', { name: 'delivery.pull' }));
    const dialog = screen.getByRole('dialog', { name: 'delivery.pull' });
    fireEvent.change(within(dialog).getByLabelText('delivery.remote'), { target: { value: 'origin' } });
    fireEvent.change(within(dialog).getByLabelText('delivery.branch'), { target: { value: 'main' } });
    fireEvent.click(within(dialog).getByRole('button', { name: 'gitPanel.previewTarget' })); await act(async () => {});
    fireEvent.click(within(dialog).getByRole('button', { name: 'delivery.pull' }));
  }
  fireEvent.click(await screen.findByRole('button', { name: 'workbenchFiles.saveAndContinue' }));
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'save_development_file')).toBe(true));
  expect(invoke.mock.calls.some(([command]) => command === 'read_development_git')).toBe(false);
  review = { ...review, snapshot_token: 'after-save-snapshot', review_token: 'after-save-authority', files: [{ path: 'src/file.ts', index: 'M', worktree: 'M' }], file_versions: { 'src/file.ts': 'saved-hash' } };
  await act(async () => { saved.resolve({ path: 'src/file.ts', text: 'approved saved draft', hash: 'saved-hash', size: 20, reason: null }); });
  await waitFor(() => expect(screen.getAllByRole('alert').some(node => node.textContent === 'dev.error.git_review_stale')).toBe(true));
  expect(mutations()).toEqual([]);
  expect(changed).not.toHaveBeenCalled();
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'src/file.ts', '/workspace')]).toMatchObject({ draft: 'approved saved draft', file: { hash: 'saved-hash' } });
  expect(invoke.mock.calls.findIndex(([command]) => command === 'read_development_git')).toBeGreaterThan(invoke.mock.calls.findIndex(([command]) => command === 'save_development_file'));

  if (kind === 'restore') {
    fireEvent.click(screen.getByRole('button', { name: 'gitPanel.restore' })); await act(async () => {});
    expect(confirmation.mock.calls.filter(([options]) => options.title === 'gitPanel.restore')).toHaveLength(2);
    const writes = invoke.mock.calls.filter(([command]) => command === 'execute_development_git');
    expect(writes).toHaveLength(1);
    expect(writes[0][1].input).toMatchObject({ review_token: 'after-save-authority', action: { kind: 'restore', paths: ['src/file.ts'], confirmed: true } });
    expect(changed).toHaveBeenCalledTimes(1);
  }
});

it('retains new typing made while the post-guard Git review is pending', async () => {
  const original = invoke.getMockImplementation()!, pending = Promise.withResolvers<GitReview>();
  invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === 'read_development_file') return { path: 'src/file.ts', text: 'base', hash: 'baseline', size: 4, reason: null };
    if (command === 'read_development_git') return pending.promise;
    return original(command, args);
  });
  await useDevelopmentEditor.getState().open('p', 't', 'src/file.ts', '/workspace');
  render(<GitPanel {...props} />); await act(async () => {});
  fireEvent.change(screen.getByLabelText('gitPanel.branch'), { target: { value: 'topic' } });
  await waitFor(() => expect(invoke.mock.calls.some(([command]) => command === 'read_development_git')).toBe(true));
  act(() => { useDevelopmentEditor.getState().edit('typed during rereview'); });
  await act(async () => { pending.resolve(review); });
  expect(mutations()).toEqual([]);
  expect(useDevelopmentEditor.getState().documents[documentId('p', 't', 'src/file.ts', '/workspace')].draft).toBe('typed during rereview');
  expect(screen.getByLabelText('gitPanel.branch')).toHaveProperty('disabled', false);
});

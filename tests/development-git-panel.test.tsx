import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { Modal } from '@dgos/ui';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { GitPanel } from '../src/components/development/GitPanel';
import { useCommitMessageGenerator } from '../src/components/development/CommitMessageGenerator';
import { useDevelopment } from '../src/stores/developmentStore';
import { useDevelopmentDrafts } from '../src/stores/developmentDrafts';
import { developmentEditorMutationAffects } from '../src/stores/developmentEditor';
import type { ExecuteGitInput, GitReview } from '../src/stores/developmentDelivery';
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('../src/i18n', () => ({ useLocale: () => ({ locale: 'en-US', t: (key: string) => key }), t: (key: string) => key }));
const project = { id: 'p', owner: 'alice', name: 'Project', path: '/workspace', directory_identity: 'dir', archived: false, created_at: 1, last_opened_at: null };
const props = { projectId: 'p', taskId: 't', workspacePath: '/workspace', sessions: [], live: {} };
const base = (): GitReview => ({ task_id: 't', kind: 'git', path: '/workspace', branch: 'main', head: 'a'.repeat(40), files: [{ path: 'src/file.ts', index: 'M', worktree: 'M' }], index: [], conflicts: [], staged_diff: 'staged changes', worktree_diff: 'working changes', review_token: 'review', initial_staged_paths: [], merge_heads: [], baseline_version: '', file_versions: {} });
let review: GitReview;
let receipts: unknown[];
beforeEach(() => {
  localStorage.clear(); useDevelopment.getState().reset(); useDevelopment.setState({ projects: [project] }); review = base(); receipts = [];
  invoke.mockImplementation(async (command: string, args: { input: ExecuteGitInput }) => {
    if (command === 'read_development_git' || command === 'read_development_project_git') return review;
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
  review = { ...review, branch: 'topic', review_token: 'changed' };
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
  fireEvent.change(screen.getByLabelText('delivery.commitMessage'), { target: { value: 'My own message' } });
  await act(async () => { pending.resolve('Generated message'); });
  expect((screen.getByLabelText('delivery.commitMessage') as HTMLTextAreaElement).value).toBe('My own message');
  const cancelled = Promise.withResolvers<string | null>(); generate.mockImplementation(() => cancelled.promise);
  fireEvent.click(screen.getByRole('button', { name: 'gitPanel.generate' }));
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
  expect(row.firstElementChild?.textContent).toBe('fffffff'); expect(row.lastElementChild?.tagName).toBe('TIME'); expect(screen.getByText('gitPanel.historyBound')).toBeTruthy();
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

it('rejects a changed raw review even when redacted diffs and file statuses are identical', async () => {
  render(<GitPanel {...props} />); await act(async () => {});
  review = { ...review, review_token: 'different-raw-bytes', file_versions: { 'src/file.ts': 'changed-hash' } };
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

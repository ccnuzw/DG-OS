import { invoke } from '@tauri-apps/api/core';
import { useDevelopment } from './developmentStore';
import { hostCode } from './hostError';
import type { Outcome } from './developmentStore';
import type { AgentSession, Engine } from './agentRunnerStore';

export interface StoredWorkspace {
  id: string; owner: string; project_id: string; path: string; directory_identity: string;
  kind: 'original' | 'imported' | 'managed'; common_dir: string | null; common_identity: string | null;
  created_operation: string | null; base_commit: string | null;
}
export interface WorkspaceView { workspaces: StoredWorkspace[]; selected_workspace_id: string | null; operation: GitOperation | null; operations: GitOperation[] }
export interface ReadWorkspacesInput { project_id: string; task_id?: string | null }
export interface SaveWorkspaceInput {
  request_id: string; task_id: string; action: 'create' | 'import' | 'select' | 'remove';
  workspace_id?: string | null; path?: string | null; branch?: string | null; base_commit?: string | null; retain_branch?: string | null;
}
export interface GitFile { path: string; original_path?: string | null; index: string; worktree: string }
export interface GitIndexEntry { path: string; mode: string; oid: string; stage: number }
export interface GitReview {
  task_id: string; kind: 'git' | 'directory'; path: string; common_dir?: string | null; common_identity?: string | null;
  branch: string | null; head: string | null; merge_heads: string[]; files: GitFile[]; index: GitIndexEntry[];
  conflicts: GitIndexEntry[]; snapshot_token: string; review_token: string; baseline_version: string;
  initial_staged_paths: string[]; file_versions: Record<string, string>;
}
export interface GitScopeInput { project_id: string; task_id: string | null }
export interface GitStagedDiff { snapshot_token: string; diff: string }
export interface GitHistoryInput { task_id: string; start_oid?: string | null; cursor?: string | null; limit: number }
export interface GitHistoryEntry { oid: string; author: string; timestamp: number; subject: string; parents: string[] }
export interface GitHistoryPage { anchor: string; next_cursor: string | null; commits: GitHistoryEntry[] }
export interface GitPullPreview { review_token: string; remote: string; branch: string; remote_ref: string; remote_display: string; preview_token: string }
export type GitAction = { kind: 'stage' | 'unstage' | 'untrack'; paths: string[] } | { kind: 'restore'; paths: string[]; confirmed: boolean } | { kind: 'restore_hunk'; path: string; diff_hash: string; hunk_index: number; confirmed: boolean } | { kind: 'commit'; message: string; confirm_full_index: boolean; confirm_preexisting_staged: boolean } | { kind: 'push'; remote: string; trust_directory: boolean; trust_network: boolean } | { kind: 'pull'; remote: string; branch: string; preview_token: string; trust_directory: boolean; trust_network: boolean } | { kind: 'switch' | 'merge'; branch: string } | { kind: 'discard'; baseline_version: string; files: { path: string; expected_hash: string }[]; confirmed: boolean };
export interface ExecuteGitInput { id: string; task_id: string; review_token: string; action: GitAction }
export interface GitOperation { id: string; kind: string; status: 'pending' | 'succeeded' | 'failed' | 'unknown'; project_id: string; task_id: string; workspace_id: string | null; request_fingerprint: string; details: { request?: unknown; before?: unknown; after?: unknown; error?: string | null; files?: unknown } }
export interface ReconcileGitInput { id: string; trust_directory: boolean; trust_network: boolean }
export interface StoredHandoff {
  id: string; owner: string; project_id: string; task_id: string; source_session_id: string; source_run_id?: string | null;
  target_engine: Engine; permission_epoch: number; summary: string; fingerprint: string; target_session_id?: string | null; created_at: number;
}
export interface HandoffView { handoff: StoredHandoff; session: { id: string; owner: string; project_id: string | null; task_id: string | null; view: AgentSession } }
export interface PrepareHandoffInput { task_id: string; source_session_id: string; target_engine: Engine; remaining: string }
export interface AcceptHandoffInput {
  handoff_id: string; summary: string; confirmed: true; project_revision: number;
  provider: { provider_id: string; model_id: string; provider_revision: number };
  options: { workspace: string; resume_id: null; executable: null; model: null; trust_workspace: true };
}

async function call<T>(command: string, args: Record<string, unknown>): Promise<Outcome<T>> {
  try { return { ok: true, value: await invoke<T>(command, args) }; }
  catch (error) { return { ok: false, code: hostCode(error) }; }
}
export function readDevelopmentWorkspaces(input: ReadWorkspacesInput) { return call<WorkspaceView>('read_development_workspaces', { input }); }
export function saveDevelopmentWorkspace(input: SaveWorkspaceInput) { return call<WorkspaceView>('save_development_workspaces', { input }); }
export function readDevelopmentGit(taskId: string) { return call<GitReview>('read_development_git', { taskId }); }
export function readDevelopmentStagedDiff(input: GitScopeInput) { return call<GitStagedDiff>('read_development_staged_diff', { input }); }
export function readDevelopmentGitHistory(input: GitHistoryInput) { return call<GitHistoryPage>('read_development_git_history', { input }); }
export function previewDevelopmentGitPull(taskId: string, remote: string, branch: string) { return call<GitPullPreview>('preview_development_git_pull', { input: { task_id: taskId, remote, branch } }); }
export function executeDevelopmentGit(input: ExecuteGitInput) { return call<GitOperation>('execute_development_git', { input }); }
export function reconcileDevelopmentGit(input: ReconcileGitInput) { return call<GitOperation>('reconcile_development_git', { input }); }
export function prepareDevelopmentHandoff(input: PrepareHandoffInput) { return call<StoredHandoff>('prepare_development_handoff', { input }); }
export function listDevelopmentHandoffs(taskId: string) { return call<StoredHandoff[]>('list_development_handoffs', { taskId }); }
export function acceptDevelopmentHandoff(input: AcceptHandoffInput) { return call<HandoffView>('accept_development_handoff', { input }); }

// Only request identity is durable in the renderer. Results belong to the host ledger.
type RequestIdentity = Pick<GitOperation, 'id' | 'kind' | 'project_id' | 'task_id'>;
function requestKey(owner: string) { return `dgos.delivery.requests:${owner}`; }
export function deliveryRequests(owner: string): RequestIdentity[] {
  const raw = localStorage.getItem(requestKey(owner));
  if (!raw) return [];
  const requests: unknown = JSON.parse(raw);
  if (!Array.isArray(requests)) throw new Error('invalid_request');
  return requests.filter((item): item is RequestIdentity => item && ['id', 'kind', 'project_id', 'task_id'].every(key => typeof item[key] === 'string'));
}
function rememberRequest(input: RequestIdentity): void {
  const owner = useDevelopment.getState().projects.find(item => item.id === input.project_id)?.owner;
  if (!owner) throw new Error('project_not_found');
  localStorage.setItem(requestKey(owner), JSON.stringify([...deliveryRequests(owner), input]));
}
export function forgetDeliveryRequests(owner: string, receipts: GitOperation[]): void {
  const terminal = new Set(receipts.filter(item => item.status === 'succeeded' || item.status === 'failed').map(item => item.id));
  localStorage.setItem(requestKey(owner), JSON.stringify(deliveryRequests(owner).filter(item => !terminal.has(item.id))));
}
export async function dispatchDeliveryRequest<T>(identity: RequestIdentity, dispatch: () => Promise<Outcome<T>>): Promise<Outcome<T>> {
  try { rememberRequest(identity); } catch (error) { return { ok: false, code: hostCode(error) }; }
  const epoch = useDevelopment.getState().epoch;
  const result = await dispatch();
  if (!result.ok && ['invalid_request', 'invalid_request_id', 'invalid_workspace', 'invalid_branch', 'invalid_commit_message', 'task_not_found', 'project_not_found', 'project_archived', 'workspace_immutable', 'workspace_busy', 'workspace_dirty', 'workspace_in_use', 'git_review_stale', 'git_unmerged', 'git_empty_commit', 'git_full_index_confirmation_required', 'git_preexisting_staged_confirmation_required', 'git_worktree_dirty', 'git_detached_head', 'git_unborn_head', 'git_pull_trust_required', 'git_pull_preview_stale', 'unsafe_git_remote', 'invalid_git_ref', 'invalid_git_remote', 'invalid_git_paths', 'invalid_git_hunk', 'restore_confirmation_required', 'file_sensitive', 'file_access_denied', 'file_binary', 'file_too_large', 'git_special_entry', 'git_review_incomplete'].includes(result.code)) {
    const owner = useDevelopment.getState().projects.find(item => item.id === identity.project_id)?.owner;
    if (epoch === useDevelopment.getState().epoch && owner) forgetDeliveryRequests(owner, [{ ...identity, status: 'failed', workspace_id: null, request_fingerprint: '', details: {} }]);
  }
  if (epoch === useDevelopment.getState().epoch) await useDevelopment.getState().load();
  return result;
}


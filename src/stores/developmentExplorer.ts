import { invoke } from '@tauri-apps/api/core';
import { hostCode } from './hostError';
import type { Outcome } from './developmentStore';
import type { GitHistoryPage, GitReview } from './developmentDelivery';
import type { DevDiff } from './developmentReview';

export interface SessionSummary { session_id: string; preview: string | null; last_activity_at: number | null }
export interface CreateEntryInput { project_id: string; task_id?: string | null; path: string; kind: 'file' | 'directory'; trust_directory: boolean }
export interface CreatedEntry { path: string; kind: 'file' | 'directory' }
/** Read-only original-workspace data: deliberately carries no task review token. */
export type ProjectGitOverview = Pick<GitReview, 'kind' | 'path' | 'branch' | 'head' | 'files' | 'conflicts' | 'staged_diff' | 'worktree_diff'>;
export interface ProjectDiffInput { project_id: string; path: string; source: 'index' | 'worktree' }
export interface ProjectGitHistoryInput { project_id: string; start_oid?: string | null; cursor?: string | null; limit: number }

async function call<T>(command: string, args: Record<string, unknown>): Promise<Outcome<T>> {
  try { return { ok: true, value: await invoke<T>(command, args) }; }
  catch (error) { return { ok: false, code: hostCode(error) }; }
}
export function readDevelopmentSessionSummaries(): Promise<Outcome<SessionSummary[]>> {
  return call('read_development_session_summaries', {});
}
/** Exclusive creation. An unknown result is inspect-only, never automatically retried. */
export function createDevelopmentEntry(input: CreateEntryInput): Promise<Outcome<CreatedEntry>> {
  return call('create_development_entry', { input });
}
export function readDevelopmentProjectGit(projectId: string): Promise<Outcome<ProjectGitOverview>> {
  return call('read_development_project_git', { projectId });
}
export function readDevelopmentProjectDiff(input: ProjectDiffInput): Promise<Outcome<DevDiff>> {
  return call('read_development_project_diff', { input });
}
export function readDevelopmentProjectGitHistory(input: ProjectGitHistoryInput): Promise<Outcome<GitHistoryPage>> {
  return call('read_development_project_git_history', { input });
}

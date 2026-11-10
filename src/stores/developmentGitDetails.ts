import { invoke } from '@tauri-apps/api/core';
import { hostCode } from './hostError';
import type { Outcome } from './developmentStore';
import type { GitHistoryEntry } from './developmentDelivery';

export interface GitFileStats { path: string; index_added: number | null; index_deleted: number | null; worktree_added: number | null; worktree_deleted: number | null }
export interface GitDetails {
  branches: string[]; remotes: string[]; remote_branches: Record<string, string[]>; remote_push_targets: Record<string, string[]>; remote_fetch_targets: Record<string, string[]>;
  upstream: string | null; ahead: number | null; behind: number | null;
  files: GitFileStats[]; history: GitHistoryEntry[]; history_truncated: boolean;
}
export interface GitDetailsInput { project_id: string; task_id: string | null; path?: string | null }
export async function readDevelopmentGitDetails(input: GitDetailsInput): Promise<Outcome<GitDetails>> {
  try { return { ok: true, value: await invoke<GitDetails>('read_development_git_details', { input }) }; }
  catch (error) { return { ok: false, code: hostCode(error) }; }
}

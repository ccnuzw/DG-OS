import { invoke } from '@tauri-apps/api/core';
import { hostCode } from './hostError';
import type { Outcome } from './developmentStore';
import type { DevDiff } from './developmentReview';

export interface GitCommitFile { path: string; original_path: string | null; status: string; added: number | null; deleted: number | null }
export interface GitCommitDetails { oid: string; author: string; author_email: string; timestamp: number; message: string; parents: string[]; files: GitCommitFile[]; truncated: boolean }
export interface GitCommitInput { project_id: string; task_id: string | null; oid: string }
export async function readDevelopmentGitCommit(input: GitCommitInput): Promise<Outcome<GitCommitDetails>> {
  try { return { ok: true, value: await invoke<GitCommitDetails>('read_development_git_commit', { input }) }; }
  catch (error) { return { ok: false, code: hostCode(error) }; }
}
export async function readDevelopmentGitCommitDiff(input: GitCommitInput & { path: string }): Promise<Outcome<DevDiff>> {
  try { return { ok: true, value: await invoke<DevDiff>('read_development_git_commit_diff', { input }) }; }
  catch (error) { return { ok: false, code: hostCode(error) }; }
}

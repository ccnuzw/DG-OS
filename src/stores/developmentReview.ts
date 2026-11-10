import { invoke } from '@tauri-apps/api/core';
import { hostCode } from './hostError';
import type { AgentEvent, Engine } from './agentRunnerStore';
import type { Outcome } from './developmentStore';
import type { SessionConfigurationView } from './developmentCapabilities';

// Host APIs for task review. DTOs preserve the host's snake_case fields; failures are normalized so raw host text
// never reaches the screen. Host execution history is shown separately from native Agent event evidence.

export type FileKind = 'directory' | 'file' | 'symlink' | 'blocked';
export interface DevFileEntry { path: string; name: string; kind: FileKind; size: number; ignored: boolean }
export interface DevFileList { entries: DevFileEntry[]; truncated: boolean }
/** `text` is null when the host refuses the content; `reason` then names why (file_sensitive, file_binary, ...). */
export interface DevFileView { path: string; text: string | null; reason: string | null; hash: string | null; size: number }

export type ChangeKind = 'added' | 'modified' | 'deleted' | 'pre_existing';
/** `uncertain` means the host cannot tell whether the agent made the change; `external` means it changed outside runs. */
export interface DevReviewChange { path: string; kind: ChangeKind; pre_existing: boolean; origin: string; previewable: boolean }
export interface DevReview {
  task_id: string;
  /** Unix seconds when the baseline was captured, or null when no baseline exists yet. */
  captured_at: number | null;
  baseline_late: boolean;
  truncated: boolean;
  fingerprint: string;
  changes: DevReviewChange[];
  runs_stale: boolean;
  excluded?: { path: string; reason: string }[];
}
export interface DevDiff { path: string; before: string | null; after: string | null; diff: string; diff_hash?: string | null; reason: string | null }

export type RunStatus = 'running' | 'waiting' | 'completed' | 'failed' | 'cancelled' | 'unknown';
export interface DevRun {
  id: string; session_id: string | null; task_id: string; engine: Engine | 'host'; source: string; status: RunStatus;
  started_at: number; finished_at: number | null; summary: string; reason: string | null;
  events: (AgentEvent & { run: string | null })[]; fingerprint: string; truncated: boolean;
  configuration?: SessionConfigurationView | null;
}

async function read<T>(command: string, args: { [key: string]: unknown }): Promise<Outcome<T>> {
  try {
    return { ok: true, value: await invoke<T>(command, args) };
  } catch (error) {
    return { ok: false, code: hostCode(error) };
  }
}

export interface FileListInput { project_id: string; task_id?: string | null; path: string; query: string; include_ignored: boolean }
export function listDevelopmentFiles(input: FileListInput): Promise<Outcome<DevFileList>> {
  return read<DevFileList>('list_development_files', { input });
}

export function readDevelopmentFile(input: { project_id: string; task_id?: string | null; path: string }): Promise<Outcome<DevFileView>> {
  return read<DevFileView>('read_development_file', { input });
}

export function readDevelopmentReview(taskId: string): Promise<Outcome<DevReview>> {
  return read<DevReview>('read_development_review', { taskId });
}

export function readDevelopmentDiff(input: { task_id: string; path: string; source?: 'task' | 'index' | 'worktree' }): Promise<Outcome<DevDiff>> {
  return read<DevDiff>('read_development_diff', { input });
}

export async function listDevelopmentRuns(taskId: string): Promise<Outcome<DevRun[]>> {
  const result = await read<DevRun[]>('list_development_runs', { taskId });
  return result.ok ? { ok: true, value: result.value.filter(run => run.engine !== 'host') } : result;
}

import { invoke } from '@tauri-apps/api/core';
import { hostCode } from './hostError';
import type { DevelopmentSession, Outcome } from './developmentStore';
import { listDevelopmentRuns } from './developmentReview';
import type { AgentEvent, Json } from './agentRunnerStore';
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

export interface ConversationSearchMatch { sessionId: string; runId: string; seq: number; text: string }
export interface ConversationSearchResult { matches: ConversationSearchMatch[]; truncated: boolean; unavailable: number }

function messageText(event: AgentEvent): string | null {
  const data = event.data && typeof event.data === 'object' && !Array.isArray(event.data) ? event.data : {};
  const object = (value: Json) => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
  // Search dialogue only: tool output, approval data and hidden reasoning are not messages.
  if (!['userMessage', 'userSteering', 'agentMessage'].includes(event.kind) && !['user', 'assistant'].includes(String(data.type))) return null;
  if (typeof data.content === 'string') return data.content;
  if (Array.isArray(data.content)) {
    // A tool-use/result, image or thinking block cannot become prose through event.text fallback.
    return data.content.filter(block => object(block).type === 'text').map(block => object(block).text).filter((value): value is string => typeof value === 'string').join('\n\n') || null;
  }
  return event.text;
}

/** Uses durable task runs, not runtime attachment. Cancellation stops queued IPC and ignores in-flight results. */
export async function searchDevelopmentMessages(sessions: readonly DevelopmentSession[], query: string, signal: AbortSignal): Promise<ConversationSearchResult> {
  const needle = query.trim().toLocaleLowerCase();
  const result: ConversationSearchResult = { matches: [], truncated: false, unavailable: sessions.filter(session => !session.task_id).length };
  if (!needle || signal.aborted) return result;
  const sessionIds = new Set(sessions.map(session => session.id));
  const taskIds = [...new Set(sessions.flatMap(session => session.task_id ? [session.task_id] : []))];
  const matched = new Set<string>();
  let cursor = 0;
  await Promise.all(Array.from({ length: Math.min(3, taskIds.length) }, async () => {
    while (!signal.aborted && cursor < taskIds.length) {
      const taskId = taskIds[cursor++];
      const runs = await listDevelopmentRuns(taskId);
      if (signal.aborted) return;
      if (!runs.ok) { result.unavailable += sessions.filter(session => session.task_id === taskId).length; continue; }
      for (const run of runs.value) {
        if (!run.session_id || !sessionIds.has(run.session_id)) continue;
        result.truncated ||= run.truncated;
        if (matched.has(run.session_id)) continue;
        for (const event of run.events) {
          const text = messageText(event);
          if (!text?.toLocaleLowerCase().includes(needle)) continue;
          result.matches.push({ sessionId: run.session_id, runId: run.id, seq: event.seq, text });
          matched.add(run.session_id);
          break;
        }
      }
    }
  }));
  return result;
}
/** Exclusive creation. An unknown result is inspect-only, never automatically retried. */
export function createDevelopmentEntry(input: CreateEntryInput): Promise<Outcome<CreatedEntry>> {
  return call('create_development_entry', { input });
}
export interface EntryInput { project_id: string; task_id: string | null; path: string }
export interface InspectedEntry extends CreatedEntry { version: string }
export interface MutateEntryInput extends EntryInput { expected_version: string; action: 'copy' | 'rename' | 'delete'; destination: string | null; trust_directory: true }
export function inspectDevelopmentEntry(input: EntryInput): Promise<Outcome<InspectedEntry>> {
  return call('inspect_development_entry', { input });
}
export function mutateDevelopmentEntry(input: MutateEntryInput): Promise<Outcome<CreatedEntry>> {
  return call('mutate_development_entry', { input });
}
export function openDevelopmentEntryDirectory(input: EntryInput): Promise<Outcome<void>> {
  return call('open_development_entry_directory', { input });
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

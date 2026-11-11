import { invoke } from '@tauri-apps/api/core';
import type { AgentEvent, AgentSession, Json } from '../../stores/agentRunnerStore';
import { readDevelopmentWorkspaces, type StoredWorkspace, type WorkspaceView } from '../../stores/developmentDelivery';
import { developmentDraftKey, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import type { DevelopmentSession, DevelopmentTask, Outcome, Project } from '../../stores/developmentStore';
import { hostCode } from '../../stores/hostError';
import { isReasoningEvent } from './codexTranscript';

export interface RecoveryTarget {
  project: Project;
  task: DevelopmentTask;
  workspace: StoredWorkspace | null;
  reason: 'archived' | 'differentWorkspace' | 'workspaceUnavailable' | null;
  error: string | null;
}

/** A task without an explicit binding uses the host's original workspace, never a renderer path fallback. */
export function recoveryWorkspace(view: WorkspaceView): StoredWorkspace | null {
  return view.workspaces.find(item => view.selected_workspace_id ? item.id === view.selected_workspace_id : item.kind === 'original') ?? null;
}

export async function readRecoveryTarget(session: DevelopmentSession, project: Project, task: DevelopmentTask): Promise<RecoveryTarget> {
  const base = { project, task, workspace: null, error: null };
  if (project.owner !== session.owner || task.owner !== session.owner || task.project_id !== project.id) return { ...base, reason: 'workspaceUnavailable' };
  if (project.archived) return { ...base, reason: 'archived' };
  const result = await readDevelopmentWorkspaces({ project_id: project.id, task_id: task.id });
  if (!result.ok) return { ...base, reason: 'workspaceUnavailable', error: result.code };
  const workspace = recoveryWorkspace(result.value);
  if (!workspace || workspace.owner !== session.owner || workspace.project_id !== project.id) return { ...base, reason: 'workspaceUnavailable' };
  return { ...base, workspace, reason: workspace.path === session.workspace ? null : 'differentWorkspace' };
}

/** Observation only: no runner attachment, native resume, trust grant, or approval response. */
export async function readRecoverySession(session: DevelopmentSession): Promise<Outcome<AgentSession>> {
  try {
    const value = await invoke<AgentSession>('get_agent_session', { sessionId: session.id });
    if (value.id !== session.id || value.engine !== session.engine || value.workspace !== session.workspace) return { ok: false, code: 'session_invalid' };
    return { ok: true, value };
  } catch (error) { return { ok: false, code: hostCode(error) }; }
}

const object = (value: Json): Record<string, Json> => value && typeof value === 'object' && !Array.isArray(value) ? value : {};
export interface RecoveryMessage { seq: number; role: 'user' | 'assistant'; text: string }
/** The recovery browser shows saved dialogue only, not tool output, approval payloads, or hidden reasoning. */
export function recoveryMessages(events: readonly AgentEvent[]): RecoveryMessage[] {
  return events.flatMap(event => {
    if (isReasoningEvent(event)) return [];
    const data = object(event.data);
    const user = ['userMessage', 'userSteering'].includes(event.kind) || data.type === 'user';
    const assistant = event.kind === 'agentMessage' || data.type === 'assistant';
    if (!user && !assistant) return [];
    const text = typeof data.content === 'string' ? data.content : Array.isArray(data.content)
      ? data.content.filter(block => object(block).type === 'text').map(block => object(block).text).filter((value): value is string => typeof value === 'string').join('\n\n')
      : event.text;
    return text ? [{ seq: event.seq, role: user ? 'user' as const : 'assistant' as const, text }] : [];
  });
}

/** Re-key the same session's unsent text only after host-confirmed classification; never drop another draft. */
export function preserveClassifiedConversationDraft(source: DevelopmentSession, saved: DevelopmentSession): void {
  if (source.id !== saved.id || source.owner !== saved.owner || source.project_id === saved.project_id || !saved.project_id) return;
  const from = developmentDraftKey('conversation', source.project_id, source.id, 'message');
  const to = developmentDraftKey('conversation', saved.project_id, saved.id, 'message');
  useDevelopmentDrafts.setState(state => {
    const entry = state.entries[from], target = state.entries[to];
    if (!entry || typeof entry.value !== 'string' || target && target.value !== entry.value) return state;
    const entries = { ...state.entries, [to]: target ?? entry };
    delete entries[from];
    return { entries };
  });
}

export interface RecoveryDraftConflict { sourceKey: string; targetKey: string; original: string; current: string }
export function unclassifiedRecoveryDraft(session: DevelopmentSession, entries = useDevelopmentDrafts.getState().entries): RecoveryDraftConflict | null {
  if (!session.project_id || !session.task_id) return null;
  const sourceKey = developmentDraftKey('conversation', null, session.id, 'message');
  const targetKey = developmentDraftKey('conversation', session.project_id, session.id, 'message');
  const source = entries[sourceKey], target = entries[targetKey];
  if (!source || typeof source.value !== 'string' || !source.value || target && typeof target.value !== 'string') return null;
  return { sourceKey, targetKey, original: source.value, current: typeof target?.value === 'string' ? target.value : '' };
}

/** Also used by the parent when an already-classified record has an unresolved legacy draft. */
export function hasUnclassifiedRecoveryDraft(session: DevelopmentSession): boolean {
  return Boolean(unclassifiedRecoveryDraft(session));
}

/** An explicit choice swaps the two texts; the previous current draft remains reachable in recovery. */
export function activateRecoveredConversationDraft(session: DevelopmentSession, expected: RecoveryDraftConflict): boolean {
  let applied = false;
  useDevelopmentDrafts.setState(state => {
    const conflict = unclassifiedRecoveryDraft(session, state.entries);
    if (!conflict || conflict.original !== expected.original || conflict.current !== expected.current) return state;
    const source = state.entries[conflict.sourceKey], target = state.entries[conflict.targetKey];
    const entries = { ...state.entries, [conflict.targetKey]: { ...(target ?? source), value: conflict.original } };
    if (conflict.current) entries[conflict.sourceKey] = { ...source, value: conflict.current };
    else delete entries[conflict.sourceKey];
    applied = true;
    return { entries };
  });
  return applied;
}

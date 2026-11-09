import { developmentDraftKey, useDevelopmentDrafts } from './developmentDrafts';

/** Intent only: the host re-reads and validates content at explicit send time. */
export interface ConversationReference {
  project_id: string;
  task_id: string | null;
  path: string;
  kind: 'path' | 'file' | 'selection' | 'diff';
  expected_hash?: string;
  diff_source?: 'task' | 'index' | 'worktree';
  start_line?: number;
  end_line?: number;
}
export interface CapturedConversationReference { id: string; reference: ConversationReference }
const empty: readonly CapturedConversationReference[] = [];
export const conversationReferencesKey = (projectId: string, ownerId: string) => developmentDraftKey('conversation', projectId, ownerId, 'references');

function entries(key: string): readonly CapturedConversationReference[] {
  return useDevelopmentDrafts.getState().entries[key]?.value as readonly CapturedConversationReference[] | undefined ?? empty;
}
export function useConversationReferences(projectId: string, ownerId: string) {
  const key = conversationReferencesKey(projectId, ownerId);
  return useDevelopmentDrafts(state => state.entries[key]?.value as readonly CapturedConversationReference[] | undefined ?? empty);
}
export function captureConversationReferences(projectId: string, ownerId: string): CapturedConversationReference[] {
  return entries(conversationReferencesKey(projectId, ownerId)).map(item => ({ id: item.id, reference: { ...item.reference } }));
}
export function getConversationReferences(projectId: string, ownerId: string): ConversationReference[] {
  return captureConversationReferences(projectId, ownerId).map(item => item.reference);
}
export function addConversationReference(projectId: string, ownerId: string, reference: ConversationReference): void {
  if (reference.project_id !== projectId) return;
  const copy: ConversationReference = { project_id: reference.project_id, task_id: reference.task_id, path: reference.path, kind: reference.kind, ...(reference.kind === 'diff' ? { diff_source: reference.diff_source ?? 'task' } : {}), ...(reference.expected_hash !== undefined ? { expected_hash: reference.expected_hash } : {}), ...(reference.start_line !== undefined ? { start_line: reference.start_line } : {}), ...(reference.end_line !== undefined ? { end_line: reference.end_line } : {}) };
  const key = conversationReferencesKey(projectId, ownerId);
  useDevelopmentDrafts.setState(state => {
    const before = entries(key);
    if (before.some(item => JSON.stringify(item.reference) === JSON.stringify(copy))) return state;
    return { entries: { ...state.entries, [key]: { kind: 'conversation', value: [...before, { id: crypto.randomUUID(), reference: copy }], baseline: empty } } };
  });
}
/** With a capture, remove only those identities, never references added during a send. */
export function clearConversationReferences(projectId: string, ownerId: string, captured?: readonly CapturedConversationReference[]): void {
  const key = conversationReferencesKey(projectId, ownerId);
  const ids = captured ? new Set(captured.map(item => item.id)) : null;
  useDevelopmentDrafts.setState(state => {
    const remaining = ids ? entries(key).filter(item => !ids.has(item.id)) : [];
    const next = { ...state.entries };
    if (remaining.length) next[key] = { kind: 'conversation', value: remaining, baseline: empty };
    else delete next[key];
    return { entries: next };
  });
}
/** Cut over the blank tab only after admission, retaining newly added references. */
export function acknowledgeConversationReferences(projectId: string, ownerId: string, sessionId: string, captured: readonly CapturedConversationReference[]): void {
  const source = conversationReferencesKey(projectId, ownerId);
  const target = conversationReferencesKey(projectId, sessionId);
  const consumed = new Set(captured.map(item => item.id));
  useDevelopmentDrafts.setState(state => {
    const remaining = entries(source).filter(item => !consumed.has(item.id));
    const targetEntries = source === target ? [] : entries(target).filter(item => !consumed.has(item.id));
    const moved = [...targetEntries, ...remaining.filter(item => !targetEntries.some(existing => existing.id === item.id))];
    const next = { ...state.entries };
    delete next[source];
    if (moved.length) next[target] = { kind: 'conversation', value: moved, baseline: empty };
    else delete next[target];
    return { entries: next };
  });
}

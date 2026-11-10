import type { AgentSession } from './agentRunnerStore';
import { agentDraftSettingsKey, agentSettingsKey, resolveAgentDefaults, type AgentResponsePreferences, type AgentTurnSettings } from './agentTurnSettings';
import { useDevelopmentDrafts } from './developmentDrafts';
import { useDevelopment } from './developmentStore';
import { desktopSlot, flushDesktopState, retryDesktopState, setDesktopSlot, useDesktopPersistence } from './developmentDesktop';

export interface AgentChatPreferenceScope { projectId: string | null; session?: AgentSession; sessionId?: string; draftId?: string }
export interface AgentChatDisplayPreferences { follow_edits: boolean; stream_response: boolean; code_highlighting: boolean }
export interface AgentChatPreferencesValue extends AgentChatDisplayPreferences, AgentResponsePreferences {}
export const defaultAgentResponsePreferences: AgentResponsePreferences = { language: 'follow_input', brevity: 'standard', next_step_suggestions: true };
export const defaultAgentChatDisplayPreferences: AgentChatDisplayPreferences = { follow_edits: true, stream_response: true, code_highlighting: true };
export function agentChatDisplayKey(projectId: string | null, ownerId: string) { return `agent-chat-display:${JSON.stringify([projectId, ownerId])}`; }
function keys(scope: AgentChatPreferenceScope) {
  const owner = scope.session?.id ?? scope.sessionId ?? scope.draftId ?? scope.projectId ?? 'draft';
  return { settings: scope.session ? agentSettingsKey(scope.session) : agentDraftSettingsKey(scope.projectId, owner), display: agentChatDisplayKey(scope.projectId, owner) };
}
function displayPreferences(value: unknown): AgentChatDisplayPreferences {
  const saved = value && typeof value === 'object' ? value as Partial<AgentChatDisplayPreferences> : {};
  return { follow_edits: typeof saved.follow_edits === 'boolean' ? saved.follow_edits : true, stream_response: typeof saved.stream_response === 'boolean' ? saved.stream_response : true, code_highlighting: typeof saved.code_highlighting === 'boolean' ? saved.code_highlighting : true };
}
export function agentChatPreferences(scope: AgentChatPreferenceScope): AgentChatPreferencesValue {
  const key = keys(scope);
  const proposed = useDevelopmentDrafts.getState().entries[key.settings]?.value as AgentTurnSettings | undefined;
  const response = proposed?.response_preferences ?? resolveAgentDefaults(scope.projectId, scope.session?.id ?? scope.sessionId).settings.response_preferences;
  return { ...defaultAgentResponsePreferences, ...response, ...displayPreferences(desktopSlot(key.display, null)) };
}
/** Reactive projection of the existing account-private desktop/draft stores, never a second store. */
export function useAgentChatPreferences(scope: AgentChatPreferenceScope): AgentChatPreferencesValue {
  useDevelopmentDrafts(state => state.entries[keys(scope).settings]);
  useDesktopPersistence(state => state.value);
  useDevelopment(state => state.epoch);
  return agentChatPreferences(scope);
}
export async function saveAgentChatPreferences(scope: AgentChatPreferenceScope, patch: Partial<AgentChatPreferencesValue>): Promise<boolean> {
  const epoch = useDevelopment.getState().epoch;
  const account = useDesktopPersistence.getState().owner;
  const key = keys(scope);
  const before = useDevelopmentDrafts.getState().entries[key.settings];
  const beforeDisplay = desktopSlot<unknown>(key.display, null);
  const next = { ...agentChatPreferences(scope), ...patch };
  const current = before?.value as AgentTurnSettings | undefined;
  const settings: AgentTurnSettings = { ...(current ?? resolveAgentDefaults(scope.projectId, scope.session?.id ?? scope.sessionId).settings), response_preferences: { language: next.language, brevity: next.brevity, next_step_suggestions: next.next_step_suggestions } };
  const responseChanged = 'language' in patch || 'brevity' in patch || 'next_step_suggestions' in patch;
  const displayChanged = 'follow_edits' in patch || 'stream_response' in patch || 'code_highlighting' in patch;
  const display: AgentChatDisplayPreferences = { follow_edits: next.follow_edits, stream_response: next.stream_response, code_highlighting: next.code_highlighting };
  if (responseChanged) useDevelopmentDrafts.setState(state => ({ entries: { ...state.entries, [key.settings]: { kind: 'conversation', value: settings, baseline: before?.baseline ?? {} } } }));
  if (displayChanged) setDesktopSlot(key.display, display);
  const saved = useDesktopPersistence.getState().error ? await retryDesktopState() : await flushDesktopState();
  if (epoch !== useDevelopment.getState().epoch || account !== useDesktopPersistence.getState().owner) return false;
  if (saved) return true;
  if (responseChanged) useDevelopmentDrafts.setState(state => {
    const entry = state.entries[key.settings];
    const latest = entry?.value as AgentTurnSettings | undefined;
    if (!entry || latest?.response_preferences !== settings.response_preferences) return state;
    const entries = { ...state.entries };
    if (latest === settings) { if (before) entries[key.settings] = before; else delete entries[key.settings]; }
    else {
      const restored = { ...latest };
      if (current?.response_preferences) restored.response_preferences = current.response_preferences;
      else delete restored.response_preferences;
      entries[key.settings] = { ...entry, value: restored };
    }
    return { entries };
  });
  if (displayChanged && desktopSlot(key.display, null) === display) setDesktopSlot(key.display, beforeDisplay);
  return false;
}
export function bindAgentChatPreferences(projectId: string | null, draftId: string, session: Pick<AgentSession, 'id'>): void {
  const source = agentChatDisplayKey(projectId, draftId), target = agentChatDisplayKey(projectId, session.id);
  const saved = desktopSlot<unknown>(source, null);
  if (saved !== null && desktopSlot(target, null) === null) { setDesktopSlot(target, displayPreferences(saved)); setDesktopSlot(source, null); }
}
/** Presentation only: tools, approvals, cancellation and native events remain live. */
export function buffersAgentText(session: AgentSession, run: string | null | undefined, stream: boolean): boolean {
  return !stream && Boolean(run && run === session.active_run && ['running', 'waiting'].includes(session.status) && !session.events.some(event => event.run === run && ['result', 'turn/completed'].includes(event.kind)));
}

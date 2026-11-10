import type { AgentSession } from './agentRunnerStore';
import type { CliProviderConfiguration } from './developmentCapabilities';
import { developmentDraftKey, useDevelopmentDrafts } from './developmentDrafts';
import { useDevelopment } from './developmentStore';
import { desktopSlot, setDesktopSlot } from './developmentDesktop';

export type AgentPermissionMode = 'default' | 'read_only' | 'accept_edits';
export type AgentCollaborationMode = 'default' | 'plan';
export interface AgentTurnSettings { reasoning_effort?: string | null; permission_mode?: AgentPermissionMode; collaboration_mode?: AgentCollaborationMode }
export interface AgentControls {
  reasoning: { available: boolean; options: string[]; current: string | null; source: string; reason: string | null };
  permission: { available: boolean; options: AgentPermissionMode[]; current: AgentPermissionMode | null; source: string; reason: string | null };
  collaboration?: { available: boolean; options: AgentCollaborationMode[]; current: AgentCollaborationMode | null; source: string; reason: string | null } | null;
  steering?: boolean;
}
export interface AgentDefaults { configuration: CliProviderConfiguration | null; settings: AgentTurnSettings }
export interface ResolvedAgentDefaults extends AgentDefaults { source: 'session' | 'project' | 'application' | 'none' }
const defaultsKey = (scope: string) => `agent-defaults:${scope}`;
function projectFor(session: AgentSession) {
  return useDevelopment.getState().sessions.find(item => item.id === session.id)?.project_id ?? useDevelopment.getState().selection.project_id;
}
export function agentSettingsKey(session: AgentSession) {
  return developmentDraftKey('conversation', projectFor(session), session.id, 'turn-settings');
}
export function agentDraftSettingsKey(projectId: string | null, draftId: string) {
  return developmentDraftKey('conversation', projectId, draftId, 'turn-settings');
}
/** Proposed values only; native controls and receipts remain the applied-state authority. */
export function agentTurnSettings(session: AgentSession): AgentTurnSettings | undefined {
  return useDevelopmentDrafts.getState().entries[agentSettingsKey(session)]?.value as AgentTurnSettings | undefined;
}
export function resolveAgentDefaults(projectId: string | null, sessionId?: string | null): ResolvedAgentDefaults {
  const session = sessionId ? desktopSlot<AgentDefaults | null>(`agent-session:${sessionId}`, null) : null;
  if (session) {
    const key = developmentDraftKey('conversation', projectId, sessionId!, 'turn-settings');
    const proposed = useDevelopmentDrafts.getState().entries[key]?.value as AgentTurnSettings | undefined;
    return { ...session, settings: proposed ?? session.settings, source: 'session' };
  }
  // A legacy session with no snapshot is unknown, not today's project default.
  if (sessionId) return { configuration: null, settings: {}, source: 'none' };
  const project = projectId ? desktopSlot<AgentDefaults | null>(defaultsKey(projectId), null) : null;
  if (project) return { ...project, source: 'project' };
  const application = desktopSlot<AgentDefaults | null>(defaultsKey('application'), null);
  return application ? { ...application, source: 'application' } : { configuration: null, settings: {}, source: 'none' };
}
/** Explicit user action only. A per-session change never updates these defaults. */
export function setAgentDefaults(scope: string, defaults: AgentDefaults): void {
  setDesktopSlot(defaultsKey(scope), {
    configuration: defaults.configuration ? { engine: defaults.configuration.engine, provider: { ...defaults.configuration.provider } } : null,
    // Enlarged editing permission is a session intent, not a default for another project.
    settings: { ...defaults.settings, ...(scope === 'application' && defaults.settings.permission_mode === 'accept_edits' ? { permission_mode: 'default' } : {}) },
  });
}
export function rememberAgentSessionConfiguration(_projectId: string | null, sessionId: string, configuration: CliProviderConfiguration, settings: AgentTurnSettings = {}): void {
  const key = `agent-session:${sessionId}`;
  if (desktopSlot<AgentDefaults | null>(key, null)) return;
  // Existing sessions without historical settings remain unknown; never manufacture
  // old settings from today's project default.
  setDesktopSlot(key, { configuration: { engine: configuration.engine, provider: { provider_id: configuration.provider.provider_id, model_id: configuration.provider.model_id, provider_revision: configuration.provider.provider_revision } }, settings: { ...settings } });
}
/** Freeze defaults once per draft. Later edits to a project never mutate old sessions. */
export function bindAgentDraftSettings(projectId: string | null, draftId: string, session: AgentSession): void {
  const source = agentDraftSettingsKey(projectId, draftId), target = agentSettingsKey(session);
  const inherited = resolveAgentDefaults(projectId).settings;
  useDevelopmentDrafts.setState(state => {
    if (state.entries[target]) return state;
    const value = state.entries[source]?.value as AgentTurnSettings | undefined;
    if (!value && !Object.keys(inherited).length) return state;
    const entries = { ...state.entries, [target]: { kind: 'conversation' as const, value: { ...(value ?? inherited) }, baseline: {} } };
    if (source !== target) delete entries[source];
    return { entries };
  });
}

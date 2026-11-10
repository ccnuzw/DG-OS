import type { AgentSession } from './agentRunnerStore';
import { developmentDraftKey, useDevelopmentDrafts } from './developmentDrafts';
import { useDevelopment } from './developmentStore';

export type AgentPermissionMode = 'default' | 'read_only' | 'accept_edits';
export type AgentCollaborationMode = 'default' | 'plan';
export interface AgentTurnSettings { reasoning_effort?: string | null; permission_mode?: AgentPermissionMode; collaboration_mode?: AgentCollaborationMode }
export interface AgentControls {
  reasoning: { available: boolean; options: string[]; current: string | null; source: string; reason: string | null };
  permission: { available: boolean; options: AgentPermissionMode[]; current: AgentPermissionMode | null; source: string; reason: string | null };
  collaboration?: { available: boolean; options: AgentCollaborationMode[]; current: AgentCollaborationMode | null; source: string; reason: string | null } | null;
  steering?: boolean;
}
export function agentSettingsKey(session: AgentSession) {
  const project = useDevelopment.getState().sessions.find(item => item.id === session.id)?.project_id ?? useDevelopment.getState().selection.project_id;
  return developmentDraftKey('conversation', project, session.id, session.generation, 'turn-settings');
}
// Proposed next-turn values, not an optimistic copy of native state. The send receipt and
// native controls remain the authority; rejection/unknown must not erase this intent.
export function agentTurnSettings(session: AgentSession): AgentTurnSettings | undefined {
  return useDevelopmentDrafts.getState().entries[agentSettingsKey(session)]?.value as AgentTurnSettings | undefined;
}

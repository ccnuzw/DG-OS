import type { AgentSession, AgentSkillInput } from './agentRunnerStore';
import { developmentDraftKey, useDevelopmentDrafts } from './developmentDrafts';

export type CapturedAgentSkill = { id: string; skill: AgentSkillInput };
export const agentSkillsKey = (projectId: string | null, session: Pick<AgentSession, 'id' | 'generation'>) => developmentDraftKey('conversation', projectId, session.id, 'skills');
export function captureAgentSkills(projectId: string | null, session: Pick<AgentSession, 'id' | 'generation'>): CapturedAgentSkill[] {
  return (useDevelopmentDrafts.getState().entries[agentSkillsKey(projectId, session)]?.value as CapturedAgentSkill[] | undefined) ?? [];
}
export function acknowledgeAgentSkills(projectId: string | null, session: Pick<AgentSession, 'id' | 'generation'>, submitted: CapturedAgentSkill[]) {
  const key = agentSkillsKey(projectId, session);
  useDevelopmentDrafts.setState(state => {
    const current = (state.entries[key]?.value as CapturedAgentSkill[] | undefined) ?? [];
    const remaining = current.filter(item => !submitted.some(sent => sent.id === item.id));
    if (remaining.length === current.length) return state;
    const entries = { ...state.entries };
    if (remaining.length) entries[key] = { kind: 'conversation', value: remaining, baseline: [] };
    else delete entries[key];
    return { entries };
  });
}

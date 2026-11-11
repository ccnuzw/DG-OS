import { t } from '../../i18n';
import { useDevelopment, type Outcome } from '../../stores/developmentStore';
import { prepareProjectDocumentRemoval } from '../../stores/developmentEditor';
import { guardWorkbenchDrafts, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { processActive, useDevelopmentRuntime } from '../../stores/developmentRuntime';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { confirmAction } from './shared';

export async function removeDevelopmentProject(projectId: string): Promise<Outcome<null>> {
  const development = useDevelopment.getState(), epoch = development.epoch;
  const runner = useAgentRunner.getState(), runtime = useDevelopmentRuntime.getState();
  const sessions = development.sessions.filter(session => session.project_id === projectId && runner.live[session.id] && runner.live[session.id] !== 'closed');
  const processes = [...new Map([...runtime.processes, ...Object.values(runtime.details)].map(process => [process.id, process])).values()].filter(process => process.project_id === projectId && processActive(process));
  const affected = [
    ...sessions.map(session => `${session.engine === 'codex' ? 'Codex' : 'Claude'} · ${development.sessionNames[session.id] || session.id.slice(0, 8)} · ${t(`agent.state.${runner.live[session.id]}`)}`),
    ...processes.map(process => `${process.program} · ${process.cwd}`),
  ];
  const content = [t('dev.unlinkConfirm'), ...(affected.length ? [t('dev.unlinkRunning'), ...affected] : [])].join('\n');
  if (!await confirmAction(t('dev.unlink'), content, t('dev.unlinkAndStop')) || epoch !== useDevelopment.getState().epoch || !await guardWorkbenchDrafts(projectId)) return { ok: false, code: 'cancelled' };
  const entries = useDevelopmentDrafts.getState().entries, runtimeDrafts = useDevelopmentRuntime.getState().drafts;
  const documents = await prepareProjectDocumentRemoval(projectId);
  if (!documents?.valid() || epoch !== useDevelopment.getState().epoch || entries !== useDevelopmentDrafts.getState().entries || runtimeDrafts !== useDevelopmentRuntime.getState().drafts) return { ok: false, code: 'cancelled' };
  const result = await useDevelopment.getState().updateProject(projectId, 'unlink', null);
  if (!result.ok || epoch !== useDevelopment.getState().epoch) return result;
  documents.discard();
  useDevelopmentDrafts.setState(state => ({
    entries: Object.fromEntries(Object.entries(state.entries).filter(([key, draft]) => JSON.parse(key)[2] !== projectId || entries[key] !== draft)),
    handoffs: Object.fromEntries(Object.entries(state.handoffs).filter(([key]) => JSON.parse(key)[2] !== projectId)),
  }));
  useDevelopmentRuntime.setState(state => ({ drafts: Object.fromEntries(Object.entries(state.drafts).filter(([key, draft]) => JSON.parse(key)[1] !== projectId || runtimeDrafts[key] !== draft)) }));
  return result;
}

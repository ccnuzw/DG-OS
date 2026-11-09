import { t } from '../../i18n';
import { useDevelopment, type Outcome } from '../../stores/developmentStore';
import { discardDevelopmentDraft } from '../../stores/developmentEditor';
import { guardWorkbenchDrafts, useDevelopmentDrafts } from '../../stores/developmentDrafts';
import { useDevelopmentRuntime } from '../../stores/developmentRuntime';
import { confirmAction } from './shared';

export async function removeDevelopmentProject(projectId: string): Promise<Outcome<null>> {
  const epoch = useDevelopment.getState().epoch;
  if (!await confirmAction(t('dev.unlink'), t('dev.unlinkConfirm'), t('workbenchNavigation.removeProject')) || !await discardDevelopmentDraft() || !await guardWorkbenchDrafts(projectId) || epoch !== useDevelopment.getState().epoch) return { ok: false, code: 'cancelled' };
  const result = await useDevelopment.getState().updateProject(projectId, 'unlink', null);
  if (!result.ok || epoch !== useDevelopment.getState().epoch) return result;
  useDevelopmentDrafts.setState(state => ({
    entries: Object.fromEntries(Object.entries(state.entries).filter(([key]) => JSON.parse(key)[2] !== projectId)),
    handoffs: Object.fromEntries(Object.entries(state.handoffs).filter(([key]) => JSON.parse(key)[2] !== projectId)),
  }));
  useDevelopmentRuntime.setState(state => ({ drafts: Object.fromEntries(Object.entries(state.drafts).filter(([key]) => JSON.parse(key)[1] !== projectId)) }));
  return result;
}

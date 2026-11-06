import { useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useLocale } from '../i18n';
import type { WorkspaceAppEntry } from '../modules/moduleApi';
import { appPath } from './routes';
import { appUsable } from './useAppCatalog';
import type { WorkspaceNavigation } from './useWorkspaceTabs';

// Catalog entry actions share the existing URL/tab executors; this hook owns only launcher feedback.
export function useAppActions(
  { controller, onActivate }: Pick<WorkspaceNavigation, 'controller' | 'onActivate'>,
  entries: WorkspaceAppEntry[],
  preferencesLoaded: boolean,
  savePins: () => void,
) {
  const navigate = useNavigate();
  const { t } = useLocale();
  const [noticeKey, setNoticeKey] = useState<string | null>(null);
  const closeLauncher = () => { controller.dispatch({ type: 'launcher/close' }); };
  const go = (path: string) => { closeLauncher(); navigate(path); };
  const openLauncher = () => { setNoticeKey(null); controller.dispatch({ type: 'launcher/open' }); };
  const activateSurface = (id: string) => { closeLauncher(); onActivate(id); };
  const openApp = (entry: WorkspaceAppEntry) => {
    if (!appUsable(entry)) { setNoticeKey('shell.unavailable'); return; }
    const existing = controller.getSnapshot().surfaces.find(surface => surface.kind === 'app' && surface.moduleId === entry.moduleId);
    if (existing) activateSurface(existing.id);
    else go(appPath(entry.moduleId));
  };
  const pinApp = (entry: WorkspaceAppEntry) => {
    if (!preferencesLoaded) { setNoticeKey('common.loading'); return; }
    if (!appUsable(entry)) { setNoticeKey('shell.unavailable'); return; }
    const outcome = controller.dispatch({ type: 'app/pin', moduleId: entry.moduleId });
    savePins();
    setNoticeKey(outcome.result === 'limit' ? 'shell.pinLimit' : outcome.result === 'already-pinned' ? 'shell.alreadyPinned' : 'shell.pinned');
  };
  const unpin = (moduleId: string) => {
    if (!preferencesLoaded) return;
    controller.dispatch({ type: 'app/unpin', moduleId }); savePins(); setNoticeKey('shell.unpinned');
  };
  const movePin = (moduleId: string, direction: -1 | 1) => {
    if (!preferencesLoaded) return;
    controller.dispatch({ type: 'app/move-pin', moduleId, targetIndex: controller.getSnapshot().pinnedModuleIds.indexOf(moduleId) + direction }); savePins();
  };
  const dropPin = (moduleId: string, targetIndex?: number) => {
    if (!preferencesLoaded) return;
    const entry = entries.find(value => value.moduleId === moduleId);
    if (entry && appUsable(entry)) {
      pinApp(entry);
      if (targetIndex !== undefined) controller.dispatch({ type: 'app/move-pin', moduleId, targetIndex });
      savePins();
    }
    controller.dispatch({ type: 'launcher/drag-end' });
  };
  return { go, openLauncher, closeLauncher, activateSurface, openApp, pinApp, unpin, movePin, dropPin, launcherNotice: noticeKey ? t(noticeKey) : '' };
}

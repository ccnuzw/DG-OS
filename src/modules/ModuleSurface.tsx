import { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { Button, Modal } from '@dgos/ui';
import { ROUTES } from '../appRoutes';
import { useLocale } from '../i18n';
import { confirmDiscard } from '../workspace/closeCoordinator';
import type { WorkspaceSurface } from '../workspace/model';
import { useSurfaceCloseGuard, useSurfaceStatus } from '../workspace/SurfaceContext';
import { ModuleSlot, useModuleInstance } from './ModuleSlots';
import { moduleRuntimeHost, type ModuleDocumentView } from './moduleRuntimeHost';

// A workspace view owns its document, not the shared module runtime. Activity changes only
// hide the mounted surface; releasing the document is tied to the identity/availability effect.
export default function ModuleSurface({ surface, available }: { surface: Extract<WorkspaceSurface, {kind: 'app'}>; available: boolean }) {
  const instance = useModuleInstance(surface.moduleId);
  const [view, setView] = useState<ModuleDocumentView | null>(null);
  const [closePrompt, setClosePrompt] = useState<{ resolve: (value: boolean) => void; runtimeId: string } | null>(null);
  const { t } = useLocale();
  const navigate = useNavigate();
  const [dirty, setDirty] = useState(false);
  useSurfaceStatus({ dirty });
  useEffect(() => () => { closePrompt?.resolve(false); }, [closePrompt]);
  useEffect(() => {
    if (closePrompt && (!available || !instance || instance.crashed || instance.runtimeId !== closePrompt.runtimeId)) {
      closePrompt.resolve(false);
      setClosePrompt(null);
    }
  }, [closePrompt, available, instance?.runtimeId, instance?.crashed]);
  useEffect(() => {
    if (!instance || !view || !available) { setDirty(false); return; }
    let current = true;
    const update = () => { void moduleRuntimeHost.prepareViewClose(instance.runtimeId, surface.viewId).then(status => { if (current) setDirty(status.dirty); }).catch(() => {}); };
    update();
    const timer = window.setInterval(update, 500);
    return () => { current = false; clearInterval(timer); };
  }, [instance?.runtimeId, view, available, surface.viewId]);
  useEffect(() => {
    let active = true;
    setView(null);
    if (!instance || !available || instance.crashed) return;
    void moduleRuntimeHost.openView(instance.runtimeId, surface.viewId, surface.documentId).then(value => {
      if (active) setView(value);
    }).catch(() => { if (active) setView(null); });
    return () => { active = false; moduleRuntimeHost.closeView(instance.runtimeId, surface.viewId); };
  }, [instance?.runtimeId, instance?.crashed, available, surface.viewId, surface.documentId]);
  useSurfaceCloseGuard(async () => {
    if (!instance || !available) return true;
    const status = await moduleRuntimeHost.prepareViewClose(instance.runtimeId, surface.viewId);
    if (status.blocked) return false;
    if (!status.dirty) return true;
    if (!status.canSave) return await confirmDiscard();
    return new Promise<boolean>(resolve => setClosePrompt({ resolve, runtimeId: instance.runtimeId }));
  });
  if (!available || !instance || instance.crashed || !view) return <div className="p-6"><p role="status">{t('shell.unavailable')}</p><Button onClick={() => navigate(ROUTES.modules)}>{t('shell.openManagement')}</Button></div>;
  return <><ModuleSlot instance={view} />{closePrompt ? <Modal open title={t('shell.unsavedTitle')} description={t('shell.unsavedSaveConfirm')}
    onOpenChange={open => { if (!open) { closePrompt.resolve(false); setClosePrompt(null); } }}
    footer={<>
      <Button variant="secondary" onClick={() => { closePrompt.resolve(false); setClosePrompt(null); }}>{t('common.cancel')}</Button>
      <Button variant="danger" onClick={() => { closePrompt.resolve(true); setClosePrompt(null); }}>{t('shell.discard')}</Button>
      <Button onClick={() => { void moduleRuntimeHost.saveView(closePrompt.runtimeId, surface.viewId).then(saved => { if (saved) { closePrompt.resolve(true); setClosePrompt(current => current === closePrompt ? null : current); } }); }}>{t('shell.saveAndClose')}</Button>
    </>} /> : null}</>;
}

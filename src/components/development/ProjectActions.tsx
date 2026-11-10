import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button, DropdownMenu, Input, Modal, toast, type MenuItem } from '@dgos/ui';
import { Ellipsis } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment, type Project } from '../../stores/developmentStore';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { useWorkspaceTrust } from '../../stores/developmentTrust';
import { confirmAction, errorText } from './shared';
import { removeDevelopmentProject } from './removeProject';

export function ProjectActions({ project, disabled, onRefresh, onFiles, onGoals }: { project: Project; disabled: boolean; onRefresh: (projectId: string) => Promise<void>; onFiles: (projectId: string) => Promise<void>; onGoals?: (projectId: string) => void }) {
  const { t } = useLocale();
  const pinned = useDevelopment(state => state.pinnedProjectIds.includes(project.id));
  const epoch = useDevelopment(state => state.epoch);
  const trust = useWorkspaceTrust(project.id, null);
  useEffect(() => { if (trust.error) toast.error(errorText(trust.error)); }, [trust.error]);
  const [busy, setBusy] = useState(false);
  const lock = useRef(false);
  const [renameOpen, setRenameOpen] = useState(false);
  const [name, setName] = useState(project.name);
  const [failure, setFailure] = useState<string | null>(null);
  const current = () => epoch === useDevelopment.getState().epoch;
  async function run(action: () => Promise<void>) {
    if (lock.current || disabled) return;
    lock.current = true; setBusy(true); setFailure(null);
    try { await action(); }
    catch (error) { if (current()) toast.error(errorText(typeof error === 'string' ? error : 'unknown')); }
    finally { lock.current = false; if (current()) setBusy(false); }
  }
  async function archive() {
    const state = useDevelopment.getState();
    const live = useAgentRunner.getState().live;
    const candidates = state.sessions.filter(session => session.project_id === project.id && !state.archivedSessionIds.includes(session.id) && session.id !== state.selection.session_id && !state.tabs.some(tab => tab.sessionId === session.id) && (!live[session.id] || live[session.id] === 'closed')).map(session => session.id);
    if (!candidates.length) { toast.info(t('workbenchNavigation.noOldConversations')); return; }
    if (!await confirmAction(t('workbenchNavigation.archiveOld'), t('workbenchNavigation.archiveNotice', { count: candidates.length }), t('workbenchNavigation.archiveOld')) || !current()) return;
    useDevelopment.getState().archiveSessions(candidates);
    const count = candidates.filter(id => useDevelopment.getState().archivedSessionIds.includes(id)).length;
    toast.success(t('workbenchNavigation.archivedCount', { count }));
  }
  async function rename() {
    if (!name.trim() || lock.current) return;
    lock.current = true; setBusy(true); setFailure(null);
    try {
      const result = await useDevelopment.getState().updateProject(project.id, 'rename', name.trim());
      if (!current()) return;
      if (result.ok) setRenameOpen(false); else setFailure(result.code);
    } finally { lock.current = false; if (current()) setBusy(false); }
  }
  const item = (id: string, label: string, icon: string, action: () => Promise<void>, danger = false): MenuItem => ({ kind: 'item', id, label: t(label), icon, disabled: disabled || busy, danger, onSelect: () => void run(action) });
  const items: MenuItem[] = [
    item('refresh', 'workbenchNavigation.refreshProject', 'refresh', async () => { await onRefresh(project.id); if (current()) toast.success(t('workbenchNavigation.refreshed')); }),
    item('archive', 'workbenchNavigation.archiveOld', 'archive', archive),
    item('rename', 'workbenchNavigation.renameProject', 'edit', async () => { setName(project.name); setRenameOpen(true); }),
    item('pin', pinned ? 'workbenchNavigation.unpin' : 'workbenchNavigation.pin', 'pin', async () => useDevelopment.getState().setProjectPinned(project.id, !pinned)),
    item('files', 'workbenchNavigation.files', 'file', () => onFiles(project.id)),
    ...(onGoals ? [item('goals', 'desktop.goals', 'check', async () => onGoals(project.id))] : []),
    { kind: 'separator' },
    item('terminal', 'workbenchNavigation.openTerminal', 'terminal', async () => { if (await trust.grant('terminal') && current()) await invoke('open_development_project_external', { input: { project_id: project.id, target: 'terminal' } }); }),
    item('reveal', 'workbenchNavigation.revealProject', 'folder-open', () => invoke('open_development_project_external', { input: { project_id: project.id, target: 'reveal' } })),
    item('copy', 'workbenchNavigation.copyPath', 'copy', async () => { try { await navigator.clipboard.writeText(project.path); if (current()) toast.success(t('workbenchNavigation.pathCopied')); } catch { if (current()) toast.error(t('workbenchNavigation.copyFailed')); } }),
    { kind: 'separator' },
    item('remove', 'workbenchNavigation.removeProject', 'folder-minus', async () => { const result = await removeDevelopmentProject(project.id); if (!result.ok && result.code !== 'cancelled') throw result.code; }, true),
  ];
  return <>
    <DropdownMenu label={`${t('workbenchNavigation.manageProject')}: ${project.name}`} trigger={<Button size="xs" variant="ghost" className="dev-tree-icon-action" aria-label={`${t('workbenchNavigation.manageProject')}: ${project.name}`} title={t('workbenchNavigation.manageProject')} disabled={disabled || busy}><Ellipsis size={15} aria-hidden="true" /></Button>} items={items} />
    <Modal open={renameOpen} onOpenChange={open => { if (!lock.current) setRenameOpen(open); }} title={t('workbenchNavigation.renameProject')} footer={<><Button variant="secondary" disabled={busy} onClick={() => setRenameOpen(false)}>{t('dev.cancel')}</Button><Button disabled={busy || !name.trim()} onClick={() => void rename()}>{t('dev.rename')}</Button></>}>
      <Input label={t('dev.renameField')} value={name} maxLength={200} disabled={busy} onChange={event => setName(event.target.value)} onKeyDown={event => { if (event.key === 'Enter' && !event.nativeEvent.isComposing) { event.preventDefault(); void rename(); } }} />
      {failure ? <p role="alert" className="text-status-error-text">{errorText(failure)}</p> : null}
    </Modal>
  </>;
}

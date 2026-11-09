import { useEffect, useRef, useState } from 'react';
import { Button, Input } from '@dgos/ui';
import { Pencil, X } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment, type ConversationTab } from '../../stores/developmentStore';
import { useAgentRunner } from '../../stores/agentRunnerStore';

export function ConversationTabs({ onSelect, onClose, busy }: { onSelect: (tab: ConversationTab) => void; onClose: (tab: ConversationTab) => void; busy: boolean }) {
  const { t } = useLocale();
  const workspace = useDevelopment();
  const runner = useAgentRunner();
  const [renaming, setRenaming] = useState<string | null>(null);
  const [name, setName] = useState('');
  const viewport = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const list = viewport.current;
    if (!list) return;
    const reveal = () => list.querySelector<HTMLElement>('[data-selected="true"]')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    reveal();
    const observer = new ResizeObserver(reveal);
    observer.observe(list);
    return () => observer.disconnect();
  }, [workspace.activeTabId]);
  const save = () => { if (renaming && name.trim()) workspace.renameTab(renaming, name); setRenaming(null); };
  return <nav className="dev-session-tabs" aria-label={t('workbenchNavigation.sessionTabs')}>
    <div ref={viewport} className="dev-conversation-tablist" role="tablist" aria-label={t('workbenchNavigation.sessionTabs')} onKeyDown={event => {
      if (!(event.target instanceof HTMLElement) || event.target.getAttribute('role') !== 'tab') return;
      const index = workspace.tabs.findIndex(tab => tab.id === workspace.activeTabId);
      const next = event.key === 'ArrowRight' ? (index + 1) % workspace.tabs.length : event.key === 'ArrowLeft' ? (index + workspace.tabs.length - 1) % workspace.tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? workspace.tabs.length - 1 : -1;
      if (next >= 0) { event.preventDefault(); if (!busy) { onSelect(workspace.tabs[next]); event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]')[next]?.focus(); } }
      if (event.key === 'Delete' && index >= 0 && !busy) { event.preventDefault(); onClose(workspace.tabs[index]); }
    }}>
      {workspace.tabs.map(tab => {
        const title = tab.title || (tab.sessionId ? workspace.sessionNames[tab.sessionId] || workspace.tasks.find(task => task.id === tab.taskId)?.goal : '') || t('workbenchNavigation.blank');
        const snapshot = tab.sessionId ? runner.snapshots[tab.sessionId] : null;
        const status = snapshot?.pending.length ? 'workbenchNavigation.waiting' : snapshot?.status === 'running' ? 'workbenchNavigation.runningTab' : tab.sessionId && runner.unread[tab.sessionId] ? 'workbenchNavigation.newReply' : null;
        return <div key={tab.id} className="dev-conversation-tab" data-selected={workspace.activeTabId === tab.id}>
          <Button role="tab" id={`conversation-tab-${tab.id}`} aria-controls="dev-panel-agent" aria-selected={workspace.activeTabId === tab.id} tabIndex={workspace.activeTabId === tab.id ? 0 : -1} data-testid={`conversation-tab-${tab.id}`} size="sm" variant="ghost" disabled={busy} onClick={() => onSelect(tab)} title={title}>
            <span className="dev-tab-name">{title}<small>{workspace.projects.find(project => project.id === tab.projectId)?.name ?? t('dev.error.project_not_found')}</small></span>
            {status ? <span className="dev-nav-status">{t(status)}</span> : null}
          </Button>
          <Button size="xs" variant="ghost" aria-label={t('workbenchNavigation.renameTab')} title={t('workbenchNavigation.renameTab')} disabled={busy} onClick={() => { setRenaming(tab.id); setName(title); }}><Pencil size={12} aria-hidden="true" /></Button>
          <Button size="xs" variant="ghost" aria-label={t('workbenchNavigation.closeTab')} title={t('workbenchNavigation.closeTab')} disabled={busy} onClick={() => onClose(tab)}><X size={14} aria-hidden="true" /></Button>
        </div>;
      })}
    </div>
    {renaming ? <form className="dev-tab-rename" onSubmit={event => { event.preventDefault(); save(); }}><Input autoFocus label={t('workbenchNavigation.renameTab')} value={name} maxLength={120} onChange={event => setName(event.target.value)} onKeyDown={event => { if (event.key === 'Escape') setRenaming(null); }} /><Button type="submit" size="xs" disabled={!name.trim()}>{t('common.save')}</Button><Button size="xs" variant="ghost" onClick={() => setRenaming(null)}>{t('common.cancel')}</Button></form> : null}
  </nav>;
}

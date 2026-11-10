import { useEffect, useRef, useState } from 'react';
import { Button, ContextMenu, DropdownMenu, Input, Modal, toast, type MenuItem } from '@dgos/ui';
import { Circle, Ellipsis, MessageSquare, X } from 'lucide-react';
import { useLocale } from '../../i18n';
import { useDevelopment, type ConversationTab } from '../../stores/developmentStore';
import { useAgentRunner } from '../../stores/agentRunnerStore';
import { copyText } from '../chat/clipboard';
import { errorText } from './shared';

export interface ConversationTabsProps {
  onSelect: (tab: ConversationTab) => void;
  onClose: (tab: ConversationTab) => Promise<void>;
  onReveal?: (tab: ConversationTab) => Promise<void>;
  busy: boolean;
}

export function ConversationTabs({ onSelect, onClose, onReveal, busy }: ConversationTabsProps) {
  const { t } = useLocale();
  const workspace = useDevelopment();
  const runner = useAgentRunner();
  const [renaming, setRenaming] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [closing, setClosing] = useState(false);
  const lock = useRef<symbol | null>(null);
  const mounted = useRef(true);
  const busyRef = useRef(busy);
  busyRef.current = busy;
  const viewport = useRef<HTMLDivElement>(null);
  const { epoch, workspaceOwner } = workspace;
  const current = () => mounted.current && useDevelopment.getState().epoch === epoch && useDevelopment.getState().workspaceOwner === workspaceOwner;
  const blocked = busy || closing;
  const title = (tab: ConversationTab) => tab.title || (tab.sessionId ? workspace.sessionNames[tab.sessionId] || workspace.tasks.find(task => task.id === tab.taskId)?.goal : '') || t('workbenchNavigation.blank');
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; lock.current = null; };
  }, []);
  useEffect(() => { lock.current = null; setClosing(false); setRenaming(null); }, [epoch, workspaceOwner]);
  useEffect(() => {
    const list = viewport.current;
    if (!list) return;
    const reveal = () => list.querySelector<HTMLElement>('[data-selected="true"]')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
    reveal();
    const observer = new ResizeObserver(reveal);
    observer.observe(list);
    return () => observer.disconnect();
  }, [workspace.activeTabId]);

  async function close(tabs: ConversationTab[]) {
    if (!current() || busyRef.current || lock.current || !tabs.length) return;
    const ticket = Symbol();
    lock.current = ticket;
    setClosing(true);
    try {
      for (const tab of tabs) {
        if (!current() || lock.current !== ticket) break;
        const latest = useDevelopment.getState().tabs.find(item => item.id === tab.id);
        if (!latest) continue;
        await onClose(latest);
        // A retained tab means the draft confirmation was cancelled or its state is unknown.
        if (!current() || useDevelopment.getState().tabs.some(item => item.id === tab.id)) break;
      }
    } catch (error) {
      if (current()) toast.error(errorText(typeof error === 'string' ? error : 'unknown'));
    } finally {
      if (lock.current === ticket) { lock.current = null; if (current()) setClosing(false); }
    }
  }
  function rename(tab: ConversationTab) {
    if (!current() || busyRef.current || lock.current) return;
    setName(title(tab));
    setRenaming(tab.id);
  }
  function save() {
    if (!current() || busyRef.current || lock.current || !renaming || !name.trim()) return;
    useDevelopment.getState().renameTab(renaming, name);
    setRenaming(null);
  }
  async function copy(value: string) {
    if (!current()) return;
    const ok = await copyText(value);
    if (current()) { if (ok) toast.success(t('chat.copied')); else toast.error(t('chat.copyFailed')); }
  }
  async function reveal(tab: ConversationTab) {
    if (!current() || busyRef.current || lock.current || !onReveal) return;
    // Radix restores the menu trigger on close; reveal only after that focus cleanup.
    await new Promise<void>(resolve => window.requestAnimationFrame(() => resolve()));
    if (!current() || busyRef.current || lock.current) return;
    try { await onReveal(tab); }
    catch (error) { if (current()) toast.error(errorText(typeof error === 'string' ? error : 'unknown')); }
  }
  function menu(tab: ConversationTab): MenuItem[] {
    const index = workspace.tabs.findIndex(item => item.id === tab.id);
    const others = workspace.tabs.filter(item => item.id !== tab.id);
    const right = workspace.tabs.slice(index + 1);
    return [
      { kind: 'item', id: 'close', label: t('workbenchNavigation.closeTab'), icon: 'close', shortcut: 'Delete', disabled: blocked, onSelect: () => void close([tab]) },
      { kind: 'item', id: 'close-others', label: t('workbenchNavigation.closeOtherTabs'), disabled: blocked || !others.length, onSelect: () => void close(others) },
      { kind: 'item', id: 'close-right', label: t('workbenchNavigation.closeTabsRight'), disabled: blocked || !right.length, onSelect: () => void close(right) },
      { kind: 'item', id: 'close-all', label: t('workbenchNavigation.closeAllTabs'), disabled: blocked || !workspace.tabs.length, onSelect: () => void close(workspace.tabs) },
      { kind: 'separator' },
      { kind: 'item', id: 'reveal', label: t('workbenchNavigation.revealConversation'), icon: 'folder-open', disabled: blocked || !onReveal, onSelect: () => void reveal(tab) },
      { kind: 'item', id: 'rename', label: t('workbenchNavigation.renameTab'), icon: 'pencil', shortcut: 'F2', disabled: blocked, onSelect: () => rename(tab) },
      { kind: 'separator' },
      { kind: 'item', id: 'copy-title', label: t('workbenchNavigation.copyTitle'), icon: 'copy', onSelect: () => void copy(title(tab)) },
      { kind: 'item', id: 'copy-session-id', label: t('workbenchNavigation.copySessionId'), icon: 'copy', disabled: !tab.sessionId, onSelect: () => { if (tab.sessionId) void copy(tab.sessionId); } },
    ];
  }
  return <nav className="dev-session-tabs workbench-tabbar" aria-label={t('workbenchNavigation.sessionTabs')} aria-busy={closing}>
    <div ref={viewport} className="dev-conversation-tablist workbench-tablist" role="tablist" aria-label={t('workbenchNavigation.sessionTabs')} onKeyDown={event => {
      const target = event.target;
      if (!(target instanceof HTMLElement) || target.getAttribute('role') !== 'tab') return;
      const index = workspace.tabs.findIndex(tab => `conversation-tab-${tab.id}` === target.id);
      if (index < 0) return;
      if (event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10') {
        event.preventDefault();
        const bounds = target.getBoundingClientRect();
        target.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: bounds.left, clientY: bounds.bottom }));
        return;
      }
      const next = event.key === 'ArrowRight' ? (index + 1) % workspace.tabs.length : event.key === 'ArrowLeft' ? (index + workspace.tabs.length - 1) % workspace.tabs.length : event.key === 'Home' ? 0 : event.key === 'End' ? workspace.tabs.length - 1 : -1;
      if (next >= 0) { event.preventDefault(); if (!blocked) { onSelect(workspace.tabs[next]); event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]')[next]?.focus(); } }
      if (event.key === 'Delete' && !blocked) { event.preventDefault(); void close([workspace.tabs[index]]); }
      if (event.key === 'F2' && !blocked) { event.preventDefault(); rename(workspace.tabs[index]); }
    }}>
      {workspace.tabs.map(tab => {
        const name = title(tab);
        const project = workspace.projects.find(project => project.id === tab.projectId)?.name ?? t('dev.error.project_not_found');
        const snapshot = tab.sessionId ? runner.snapshots[tab.sessionId] : null;
        const status = snapshot?.pending.length ? 'workbenchNavigation.waiting' : (tab.sessionId ? runner.live[tab.sessionId] ?? snapshot?.status : null) === 'running' ? 'workbenchNavigation.runningTab' : tab.sessionId && runner.unread[tab.sessionId] ? 'workbenchNavigation.newReply' : null;
        const items = menu(tab);
        return <ContextMenu key={tab.id} items={items}><div className="dev-conversation-tab workbench-tab" data-selected={workspace.activeTabId === tab.id}>
          <Button className="workbench-tab-select" role="tab" id={`conversation-tab-${tab.id}`} aria-controls="dev-panel-agent" aria-selected={workspace.activeTabId === tab.id} tabIndex={workspace.activeTabId === tab.id ? 0 : -1} data-testid={`conversation-tab-${tab.id}`} size="sm" variant="ghost" disabled={blocked} onClick={() => onSelect(tab)} title={`${name}\n${project}${status ? `\n${t(status)}` : ''}`}>
            <MessageSquare size={14} aria-hidden="true" />
            <span className="dev-tab-name workbench-tab-title">{name}</span>
            {status ? <span className="dev-nav-status" title={t(status)}><Circle size={6} fill="currentColor" aria-hidden="true" /><span className="sr-only">{t(status)}</span></span> : null}
          </Button>
          <DropdownMenu label={t('workbenchNavigation.tabActions', { title: name })} items={items} trigger={<Button size="xs" variant="ghost" className="workbench-tab-menu" aria-label={t('workbenchNavigation.tabActions', { title: name })} disabled={blocked}><Ellipsis size={14} aria-hidden="true" /></Button>} />
          <Button className="workbench-tab-close" size="xs" variant="ghost" aria-label={t('workbenchNavigation.closeTab')} title={t('workbenchNavigation.closeTab')} disabled={blocked} onClick={() => void close([tab])}><X size={14} aria-hidden="true" /></Button>
        </div></ContextMenu>;
      })}
    </div>
    <Modal open={renaming !== null} onOpenChange={open => { if (!open) setRenaming(null); }} title={t('workbenchNavigation.renameTab')} footer={<><Button variant="ghost" onClick={() => setRenaming(null)}>{t('common.cancel')}</Button><Button disabled={blocked || !name.trim()} onClick={save}>{t('common.save')}</Button></>}>
      <form className="dev-tab-rename" onSubmit={event => { event.preventDefault(); save(); }}><Input autoFocus label={t('workbenchNavigation.renameTab')} value={name} maxLength={120} disabled={blocked} onChange={event => setName(event.target.value)} /></form>
    </Modal>
  </nav>;
}

import { useEffect, useRef, useState } from 'react';
import { ContextMenu, DropdownMenu, type MenuItem } from '@dgos/ui';
import { Ellipsis, FileText, GitCompareArrows, X } from 'lucide-react';
import { useDevelopment } from '../../stores/developmentStore';
import { useLocale } from '../../i18n';
import { useDevelopmentEditor, documentScope, documentId, documentDirty, openDevelopmentDocument, closeDevelopmentDocuments, type DocumentTab } from '../../stores/developmentEditor';
import type { ConversationReference } from '../../stores/developmentReferences';
import type { FileTarget } from './ProjectFiles';
import { FileEditor } from './FileEditor';
import { GitFileDiff, type DiffSource } from './GitFileDiff';
import './file-workspace.css';

export function FileWorkspace({ projectId, taskId, workspacePath, target, source = 'worktree', visible = true, ensureTask, onClose, onOpenFile, onReference, onChanged }: { projectId: string; taskId: string | null; workspacePath: string; target: FileTarget | null; source?: DiffSource; visible?: boolean; ensureTask?: () => Promise<string | null>; onClose(): void; onOpenFile(target: FileTarget, source?: DiffSource): void; onReference?: (reference: ConversationReference) => void; onChanged?: () => void }) {
  const { t } = useLocale();
  const state = useDevelopmentEditor(), root = useRef<HTMLElement>(null);
  const epoch = useDevelopment(state => state.epoch);
  const closeLock = useRef(false), currentScope = useRef('');
  const [closing, setClosing] = useState(false);
  const scope = documentScope(projectId, taskId, workspacePath);
  currentScope.current = scope;
  const tabs = state.tabs.filter(tab => documentScope(tab.projectId, tab.taskId, tab.workspacePath) === scope);
  const selected = tabs.find(tab => tab.id === state.activeTabs[scope]) ?? tabs[tabs.length - 1];
  useEffect(() => { if (target) openDevelopmentDocument({ projectId, taskId, workspacePath, path: target.path, mode: target.mode, line: target.line, source }); }, [projectId, taskId, workspacePath, target?.nonce]);
  useEffect(() => { if (visible) root.current?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' }); }, [selected?.id, visible]);
  function select(tab: DocumentTab) {
    if (epoch !== useDevelopment.getState().epoch || currentScope.current !== scope) return;
    useDevelopmentEditor.setState(current => ({ activeTabs: { ...current.activeTabs, [scope]: tab.id } }));
    if (tab.mode === 'file') void useDevelopmentEditor.getState().open(projectId, taskId, tab.path, workspacePath);
  }
  function focusTab(id?: string) {
    const buttons = root.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]');
    const button = id ? Array.from(buttons ?? []).find(item => item.id === `document-tab-${id}`) : root.current?.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]');
    button?.focus({ preventScroll: true });
    button?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  }
  function close(ids: string[], returnTo?: string, closePanel = false) {
    if (closeLock.current || epoch !== useDevelopment.getState().epoch || currentScope.current !== scope) return;
    const valid = () => !!root.current?.isConnected && epoch === useDevelopment.getState().epoch && currentScope.current === scope;
    const scopedIds = useDevelopmentEditor.getState().tabs.filter(tab => ids.includes(tab.id) && documentScope(tab.projectId, tab.taskId, tab.workspacePath) === scope).map(tab => tab.id);
    if (!scopedIds.length) { if (closePanel && valid()) onClose(); return; }
    closeLock.current = true;
    setClosing(true);
    // Let the menu return focus before a discard dialog captures its trigger.
    window.requestAnimationFrame(() => {
      if (!valid()) { closeLock.current = false; if (root.current) setClosing(false); return; }
      focusTab(returnTo);
      void (async () => {
        try {
          const closed = await closeDevelopmentDocuments(scopedIds);
          if (!valid()) return;
          if (closed && !useDevelopmentEditor.getState().tabs.some(tab => documentScope(tab.projectId, tab.taskId, tab.workspacePath) === scope)) onClose();
          else window.requestAnimationFrame(() => {
            if (!valid()) return;
            if (returnTo && useDevelopmentEditor.getState().tabs.some(tab => tab.id === returnTo)) focusTab(returnTo);
            else focusTab();
          });
        } finally {
          closeLock.current = false;
          if (root.current) setClosing(false);
        }
      })();
    });
  }
  function menu(tab?: DocumentTab): MenuItem[] {
    const ids = tabs.map(item => item.id), index = tab ? tabs.indexOf(tab) : -1;
    const item = (id: string, targets: string[]): MenuItem => ({ kind: 'item', id, label: t(`fileWorkspace.${id}`), disabled: closing || !targets.length, onSelect: () => close(targets, tab?.id) });
    return tab ? [item('closeCurrent', [tab.id]), item('closeOthers', ids.filter(id => id !== tab.id)), item('closeRight', ids.slice(index + 1)), { kind: 'separator' }, item('closeAll', ids)] : [item('closeAll', ids)];
  }
  return <section ref={root} id="dev-panel-documents" className="file-workspace" aria-label={t('fileWorkspace.title')} data-testid="dev-file-workspace">
    <ContextMenu items={menu()}><div className="workbench-tabbar file-workspace-tabbar">
      <div className="workbench-tablist file-workspace-tabs" role="tablist" aria-label={t('fileWorkspace.tabs')} aria-busy={closing} onKeyDown={event => {
        const button = (event.target as HTMLElement).closest<HTMLButtonElement>('[role="tab"]');
        const index = tabs.findIndex(tab => button?.id === `document-tab-${tab.id}`);
        if (index < 0 || event.altKey || event.ctrlKey || event.metaKey || event.nativeEvent.isComposing) return;
        if (event.key === 'ContextMenu' || event.shiftKey && event.key === 'F10') {
          event.preventDefault();
          const bounds = button!.getBoundingClientRect();
          button!.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: bounds.left, clientY: bounds.bottom }));
        } else if (event.key === 'Delete') {
          event.preventDefault();
          if (!event.repeat) close([tabs[index].id], tabs[index].id);
        } else if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
          event.preventDefault();
          const next = event.key === 'Home' ? 0 : event.key === 'End' ? tabs.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + tabs.length) % tabs.length;
          select(tabs[next]);
          focusTab(tabs[next].id);
        }
      }}>{tabs.map(tab => {
        const document = state.documents[documentId(projectId, taskId, tab.path, workspacePath)], dirty = tab.mode === 'file' && document && documentDirty(document), Icon = tab.mode === 'diff' ? GitCompareArrows : FileText;
        const label = [tab.path, tab.mode === 'diff' ? t('fileWorkspace.diff') : '', dirty ? t('direct.unsaved') : ''].filter(Boolean).join(', ');
        return <ContextMenu key={tab.id} items={menu(tab)}><div className="workbench-tab file-workspace-tab" data-selected={selected?.id === tab.id}>
          <button id={`document-tab-${tab.id}`} type="button" role="tab" className="workbench-tab-select" title={tab.path} aria-label={label} aria-selected={selected?.id === tab.id} aria-controls={`document-content-${tab.id}`} tabIndex={selected?.id === tab.id ? 0 : -1} onClick={() => select(tab)}><Icon size={14} aria-hidden="true" /><span className="workbench-tab-title">{tab.path.slice(tab.path.lastIndexOf('/') + 1)}</span>{tab.mode === 'diff' ? <small className="file-tab-diff">{t('fileWorkspace.diff')}</small> : null}{dirty ? <span aria-hidden="true" className="file-tab-dirty" /> : null}</button>
          <button type="button" className="workbench-tab-close" disabled={closing} aria-label={t('fileWorkspace.closeTab', { path: tab.path })} title={t('fileWorkspace.closeTab', { path: tab.path })} onClick={() => close([tab.id], tab.id)}><X size={12} aria-hidden="true" /></button>
        </div></ContextMenu>;
      })}</div>
      <div className="workbench-tab-actions">
        <DropdownMenu label={t('fileWorkspace.tabActions')} items={menu(selected)} trigger={<button type="button" className="workbench-tab-action" disabled={closing || !tabs.length} aria-label={t('fileWorkspace.tabActions')} title={t('fileWorkspace.tabActions')}><Ellipsis size={16} aria-hidden="true" /></button>} />
        <button type="button" className="workbench-tab-action" disabled={closing} aria-label={t('fileWorkspace.closePanel')} title={t('fileWorkspace.closePanel')} onClick={() => close(tabs.map(tab => tab.id), selected?.id, true)}><X size={15} aria-hidden="true" /></button>
      </div>
    </div></ContextMenu>
    {selected ? <nav className="file-workspace-breadcrumbs" aria-label={t('fileWorkspace.breadcrumbs')}>{selected.path.split('/').map((part, index) => <span key={index}>{index ? <span aria-hidden="true">/ </span> : null}{part}</span>)}</nav> : null}
    <div className="file-workspace-contents">{tabs.map(tab => <div id={`document-content-${tab.id}`} role="tabpanel" aria-labelledby={`document-tab-${tab.id}`} className="file-workspace-document" key={tab.id} hidden={tab.id !== selected?.id}>
      {tab.mode === 'file' ? <FileEditor projectId={projectId} taskId={taskId} path={tab.path} workspacePath={workspacePath} line={tab.line} visible={visible && selected?.id === tab.id} onReference={onReference} /> : <GitFileDiff projectId={projectId} taskId={taskId} workspacePath={workspacePath} path={tab.path} source={tab.source} ensureTask={ensureTask} onSourceChange={next => useDevelopmentEditor.setState(current => ({ tabs: current.tabs.map(item => item.id === tab.id ? { ...item, source: next } : item) }))} onOpenFile={() => onOpenFile({ path: tab.path, mode: 'file', nonce: Date.now() })} onReference={onReference} onChanged={onChanged} />}
    </div>)}{!tabs.length ? <p className="text-xs">{t('fileWorkspace.empty')}</p> : null}</div>
  </section>;
}

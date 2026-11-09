import { Button, DropdownMenu, Icon, Tooltip, cn, type MenuItem } from '@dgos/ui';
import type { ReactNode } from 'react';
import { NavLink, useMatch } from 'react-router-dom';
import { CollapseGlyph } from './icons';
import { TabBar, type TabBarProps } from './TabBar';
import { useLocale } from '../i18n';
import './shell.css';

export type NavEntry = {
  id: string;
  label: string;
  shortLabel?: string;
  to: string;
  icon: ReactNode;
  end?: boolean;
  run?: () => void;
};
export interface AppShellProps {
  navEntries: NavEntry[];
  navGroups?: { eyebrow: string; entries: NavEntry[] }[];
  pinnedEntries?: NavEntry[];
  collapsed?: boolean;
  autoCollapsed?: boolean;
  onToggleCollapsed?: () => void;
  onOpenLauncher?: () => void;
  launcherOpen?: boolean;
  onPinModule?: (moduleId: string, targetIndex?: number) => void;
  onUnpinModule?: (moduleId: string) => void;
  onMovePin?: (moduleId: string, direction: -1 | 1) => void;
  onDragModule?: (moduleId: string | null) => void;
  railFooter?: ReactNode;
  settingsEntry?: NavEntry;
  tabs: TabBarProps;
  railHidden?: boolean;
  draggingModuleId?: string | null;
  children: ReactNode;
}

function NavButton({ entry, collapsed, obscured = false }: { entry: NavEntry; collapsed: boolean; obscured?: boolean }) {
  const match = useMatch({ path: entry.to, end: entry.end ?? false, caseSensitive: true });
  return <Tooltip content={entry.label}><NavLink to={entry.to} end={entry.end} caseSensitive aria-label={entry.label} aria-current={obscured ? false : 'page'} onClick={event => { if (entry.run && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) { event.preventDefault(); entry.run(); } }} className={cn('workspace-nav-item', collapsed && 'is-collapsed', match && !obscured && 'is-active')}><span aria-hidden="true" className="workspace-nav-icon">{entry.icon}</span>{collapsed ? null : <span className="workspace-nav-label">{entry.shortLabel ?? entry.label}</span>}</NavLink></Tooltip>;
}

export function AppShell({ navEntries, navGroups = [], pinnedEntries = [], collapsed = false, autoCollapsed = false, onToggleCollapsed, onOpenLauncher, launcherOpen = false, onPinModule, onUnpinModule, onMovePin, onDragModule, railFooter, settingsEntry, tabs, railHidden = false, draggingModuleId = null, children }: AppShellProps) {
  const { t } = useLocale();
  const drop = (event: React.DragEvent, index?: number) => {
    if (!draggingModuleId) return;
    event.preventDefault(); event.stopPropagation();
    onPinModule?.(draggingModuleId, index); onDragModule?.(null);
  };
  return <div className="workspace-shell">
    {railHidden ? null : <aside aria-label={t('shell.rail')} className={cn('workspace-rail bg-surface-rail', collapsed && 'is-collapsed', draggingModuleId && 'workspace-pin-target')} onDragOver={event => { if (draggingModuleId) event.preventDefault(); }} onDrop={event => drop(event)}>
      <div className="workspace-brand" aria-label={t('shell.workspace')}><span aria-hidden="true" className="workspace-brand-mark">{t('shell.brandMark')}</span></div>
      <nav aria-label={t('shell.navigation')} className="workspace-navigation">
        <div role="group" aria-label={t('shell.overview')} className="workspace-nav-group">{navEntries.map(entry => <NavButton key={entry.id} entry={entry} collapsed={collapsed} obscured={launcherOpen} />)}</div>
        {navGroups.map(group => <div role="group" aria-label={group.eyebrow} key={group.eyebrow} className="workspace-nav-group">{group.entries.map(entry => <NavButton key={entry.id} entry={entry} collapsed={collapsed} obscured={launcherOpen} />)}</div>)}
        <div role="group" aria-label={t('nav.modules')} className="workspace-nav-group">
          {onOpenLauncher ? <Tooltip content={t('nav.modules')}><button type="button" aria-label={t('nav.modules')} aria-haspopup="dialog" aria-expanded={launcherOpen} onClick={onOpenLauncher} className={cn('workspace-nav-item', collapsed && 'is-collapsed', launcherOpen && 'is-active')}><span aria-hidden="true" className="workspace-nav-icon"><Icon name="menu" /></span>{collapsed ? null : <span className="workspace-nav-label">{t('nav.modules')}</span>}</button></Tooltip> : null}
          {draggingModuleId ? <span role="status" className="workspace-pin-hint">{t('shell.pinDrop')}</span> : null}
          {pinnedEntries.map((entry, index) => {
            const items: MenuItem[] = [{ kind: 'item', id: 'unpin', label: t('shell.unpinApp'), onSelect: () => onUnpinModule?.(entry.id) }, { kind: 'item', id: 'earlier', label: t('shell.moveEarlier'), disabled: index === 0, onSelect: () => onMovePin?.(entry.id, -1) }, { kind: 'item', id: 'later', label: t('shell.moveLater'), disabled: index === pinnedEntries.length - 1, onSelect: () => onMovePin?.(entry.id, 1) }];
            return <div key={entry.id} className="workspace-pinned-row" draggable onDragStart={event => { event.dataTransfer.setData('application/x-dgos-module', entry.id); event.dataTransfer.effectAllowed = 'move'; onDragModule?.(entry.id); }} onDragEnd={() => onDragModule?.(null)} onDragOver={event => { if (draggingModuleId) event.preventDefault(); }} onDrop={event => drop(event, index)}><NavButton entry={entry} collapsed={collapsed} obscured={launcherOpen} /><DropdownMenu label={entry.label} trigger={<Button variant="ghost" size="sm" iconOnly className="workspace-pin-actions" aria-label={t('shell.appActions', { title: entry.label })}><Icon name="more" size="sm" /></Button>} items={[{ kind: 'label', text: entry.label }, ...items]} /></div>;
          })}
        </div>
      </nav>
      <footer className="workspace-rail-footer">
        {settingsEntry ? <NavButton entry={settingsEntry} collapsed={collapsed} obscured={launcherOpen} /> : null}
        {railFooter ? <div className="workspace-account-footer" data-collapsed="true">{railFooter}</div> : null}
        {onToggleCollapsed ? <Button variant="ghost" size="sm" iconOnly disabled={autoCollapsed} onClick={onToggleCollapsed} aria-expanded={!collapsed} aria-label={collapsed ? t('shell.expand') : t('shell.collapse')} title={autoCollapsed ? t('shell.autoCollapsed') : collapsed ? t('shell.expand') : t('shell.collapse')} className="workspace-collapse"><CollapseGlyph className={cn('workspace-collapse-icon', collapsed && 'rotate-180')} /></Button> : null}
      </footer>
    </aside>}
    <div className="workspace-stage"><TabBar {...tabs} /><main className="workspace-main">{children}</main></div>
  </div>;
}

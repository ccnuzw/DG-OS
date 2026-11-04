import { Button, DropdownMenu, Icon, cn, type MenuItem } from '@dgos/ui';
import type { ReactNode } from 'react';
import { NavLink } from 'react-router-dom';
import { CollapseGlyph } from './icons';
import { TabBar, type TabBarProps } from './TabBar';
import { useLocale } from '../i18n';
import './shell.css';

export type NavEntry = {
  id: string;
  label: string;
  to: string;
  icon: ReactNode;
  end?: boolean;
  tone?: 'neutral' | 'ai' | 'resource' | 'tool' | 'app';
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
  onPinModule?: (moduleId: string, targetIndex?: number) => void;
  onUnpinModule?: (moduleId: string) => void;
  onMovePin?: (moduleId: string, direction: -1 | 1) => void;
  onDragModule?: (moduleId: string | null) => void;
  accountLabel?: string;
  railFooter?: ReactNode;
  settingsEntry?: NavEntry;
  tabs: TabBarProps;
  railHidden?: boolean;
  draggingModuleId?: string | null;
  children: ReactNode;
}

function NavButton({ entry, collapsed }: { entry: NavEntry; collapsed: boolean }) {
  return <NavLink to={entry.to} end={entry.end} caseSensitive title={entry.label} aria-label={entry.label} onClick={event => { if (entry.run && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) { event.preventDefault(); entry.run(); } }} data-tone={entry.tone ?? 'neutral'} className={({ isActive }) => cn('workspace-nav-item', collapsed && 'is-collapsed', isActive && 'is-active')}><span className="workspace-nav-icon">{entry.icon}</span>{collapsed ? null : <span className="workspace-nav-label">{entry.label}</span>}</NavLink>;
}

export function AppShell({ navEntries, navGroups = [], pinnedEntries = [], collapsed = false, autoCollapsed = false, onToggleCollapsed, onOpenLauncher, onPinModule, onUnpinModule, onMovePin, onDragModule, accountLabel, railFooter, settingsEntry, tabs, railHidden = false, draggingModuleId = null, children }: AppShellProps) {
  const { t } = useLocale();
  const drop = (event: React.DragEvent, index?: number) => {
    if (!draggingModuleId) return;
    event.preventDefault(); event.stopPropagation();
    onPinModule?.(draggingModuleId, index); onDragModule?.(null);
  };
  return <div className="workspace-shell">
    {railHidden ? null : <aside aria-label={t('shell.rail')} className={cn('workspace-rail', collapsed && 'is-collapsed', draggingModuleId && 'workspace-pin-target')} onDragOver={event => { if (draggingModuleId) event.preventDefault(); }} onDrop={event => drop(event)}>
      <div className="workspace-brand"><span aria-hidden="true" className="workspace-brand-mark">{t('shell.brandMark')}</span>{collapsed ? null : <span>{t('shell.workspace')}</span>}</div>
      <nav aria-label={t('shell.navigation')} className="workspace-navigation">
        <div className="workspace-nav-group"><span className={cn('workspace-nav-heading', collapsed && 'sr-only')}>{t('shell.overview')}</span>{navEntries.map(entry => <NavButton key={entry.id} entry={entry} collapsed={collapsed} />)}</div>
        {navGroups.map(group => <div key={group.eyebrow} className="workspace-nav-group"><span className={cn('workspace-nav-heading', collapsed && 'sr-only')}>{group.eyebrow}</span>{group.entries.map(entry => <NavButton key={entry.id} entry={entry} collapsed={collapsed} />)}</div>)}
        <div className="workspace-nav-group"><span className={cn('workspace-nav-heading', collapsed && 'sr-only')}>{t('nav.modules')}</span>
          {onOpenLauncher ? <button type="button" title={t('nav.modules')} aria-label={t('nav.modules')} onClick={onOpenLauncher} data-tone="app" className={cn('workspace-nav-item', collapsed && 'is-collapsed')}><span className="workspace-nav-icon"><Icon name="menu" /></span>{collapsed ? null : <span className="workspace-nav-label">{t('nav.modules')}</span>}</button> : null}
          {draggingModuleId ? <span role="status" className="workspace-pin-hint">{t('shell.pinDrop')}</span> : null}
          {pinnedEntries.map((entry, index) => {
            const items: MenuItem[] = [{ kind: 'item', id: 'unpin', label: t('shell.unpinApp'), onSelect: () => onUnpinModule?.(entry.id) }, { kind: 'item', id: 'earlier', label: t('shell.moveEarlier'), disabled: index === 0, onSelect: () => onMovePin?.(entry.id, -1) }, { kind: 'item', id: 'later', label: t('shell.moveLater'), disabled: index === pinnedEntries.length - 1, onSelect: () => onMovePin?.(entry.id, 1) }];
            return <div key={entry.id} className="workspace-pinned-row" draggable onDragStart={event => { event.dataTransfer.setData('application/x-dgos-module', entry.id); event.dataTransfer.effectAllowed = 'move'; onDragModule?.(entry.id); }} onDragEnd={() => onDragModule?.(null)} onDragOver={event => { if (draggingModuleId) event.preventDefault(); }} onDrop={event => drop(event, index)} onKeyDown={event => { if (event.altKey && ['ArrowUp', 'ArrowDown'].includes(event.key)) { event.preventDefault(); onMovePin?.(entry.id, event.key === 'ArrowUp' ? -1 : 1); } }}><NavButton entry={entry} collapsed={collapsed} /><DropdownMenu label={entry.label} trigger={<Button variant="ghost" size="xxs" iconOnly aria-label={t('shell.appActions', { title: entry.label })}><Icon name="more" size="sm" /></Button>} items={items} /></div>;
          })}
        </div>
      </nav>
      <footer className="workspace-rail-footer">
        {settingsEntry ? <NavButton entry={settingsEntry} collapsed={collapsed} /> : null}
        {railFooter ? <div className="workspace-account-footer" data-collapsed={collapsed}>{railFooter}</div> : null}
        {accountLabel ? <span className="workspace-account-label" title={accountLabel}>{collapsed ? accountLabel.slice(0, 1) : accountLabel}</span> : null}
        {onToggleCollapsed ? <Button variant="ghost" size="xxs" iconOnly={collapsed} disabled={autoCollapsed} onClick={onToggleCollapsed} aria-expanded={!collapsed} aria-label={collapsed ? t('shell.expand') : t('shell.collapse')} title={autoCollapsed ? t('shell.autoCollapsed') : collapsed ? t('shell.expand') : t('shell.collapse')} className="workspace-collapse"><CollapseGlyph className={cn('size-3.5', collapsed && 'rotate-180')} />{collapsed ? null : <span>{t('shell.collapseShort')}</span>}</Button> : null}
      </footer>
    </aside>}
    <div className="workspace-stage"><TabBar {...tabs} /><main className="workspace-main">{children}</main></div>
  </div>;
}

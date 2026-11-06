import { useEffect, useRef, useState } from 'react';
import { Button, Icon, cn } from '@dgos/ui';
import { useLocale } from '../i18n';
import type { WorkspaceSurface } from './model';
import type { SurfaceStatus } from './controller';
import { TabListPopover } from './WorkspaceTools';

export interface TabBarProps {
  surfaces: WorkspaceSurface[];
  activeSurfaceId: string | null;
  focusedSurfaceId?: string | null;
  runningModuleIds?: readonly string[];
  surfaceStatuses?: Readonly<Record<string, SurfaceStatus>>;
  onActivate: (surfaceId: string) => void;
  onClose: (surfaceId: string) => void;
  onNew?: () => void;
  onOpenLauncher?: () => void;
  onOpenCommands?: () => void;
  onToggleFocus?: (surfaceId: string) => void;
}

export function TabBar({ surfaces, activeSurfaceId, focusedSurfaceId = null, runningModuleIds = [], surfaceStatuses = {}, onActivate, onClose, onNew, onOpenLauncher, onOpenCommands, onToggleFocus }: TabBarProps) {
  const active = surfaces.find(surface => surface.id === activeSurfaceId) ?? null;
  const { t } = useLocale();
  const [roving, setRoving] = useState(activeSurfaceId);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => {
    setRoving(activeSurfaceId ?? surfaces[0]?.id ?? null);
    list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }, [activeSurfaceId, surfaces.length]);
  return <div className="workspace-tabbar">
    <div ref={list} role="tablist" aria-label={t('shell.tabs')} className="workspace-tabs-scroll">
      {surfaces.map(surface => {
        const selected = surface.id === activeSurfaceId;
        const running = surfaceStatuses[surface.id]?.running || (surface.kind === 'app' && runningModuleIds.includes(surface.moduleId));
        const dirty = surfaceStatuses[surface.id]?.dirty;
        const runningLabel = surfaceStatuses[surface.id]?.runningLabel ?? t('shell.running');
        const title = surface.kind === 'ai' ? t('nav.workbench') : surface.title;
        return <div key={surface.id} className={cn('workspace-tab', selected && 'is-active')}>
          <button type="button" role="tab" data-workspace-tab={surface.id} aria-selected={selected} tabIndex={(roving ?? activeSurfaceId ?? surfaces[0]?.id) === surface.id ? 0 : -1} title={title} onFocus={() => setRoving(surface.id)} onClick={() => onActivate(surface.id)} onKeyDown={event => {
            if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
            event.preventDefault();
            const buttons = Array.from(list.current?.querySelectorAll<HTMLButtonElement>('[role="tab"]') ?? []);
            const index = buttons.indexOf(event.currentTarget);
            const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : -1) + buttons.length) % buttons.length;
            buttons[next]?.focus(); buttons[next]?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
          }} className="workspace-tab-main"><Icon name={surface.kind === 'ai' ? 'file' : surface.kind === 'task' ? 'code' : surface.kind === 'resource' ? 'folder' : 'menu'} size="sm" /><span>{title}</span>{running ? <i className="workspace-running-dot" role="img" aria-label={runningLabel} title={runningLabel} /> : null}{dirty ? <i className="workspace-dirty-dot" role="img" aria-label={t('shell.unsavedTitle')} /> : null}</button>
          <button type="button" aria-label={t('shell.closeTab', { title })} onClick={() => onClose(surface.id)} className="workspace-tab-close"><Icon name="close" size="sm" /></button>
        </div>;
      })}
    </div>
    <div className="workspace-tab-actions">
      {onOpenLauncher ? <Button size="xxs" variant="ghost" iconOnly aria-label={t('shell.openApps')} title={t('shell.openApps')} onClick={onOpenLauncher}><Icon name="menu" size="sm" /></Button> : null}
      {surfaces.length ? <TabListPopover surfaces={surfaces} activeId={activeSurfaceId} onActivate={onActivate} onClose={onClose} label={t('shell.tabList')} /> : null}
      {onOpenCommands ? <Button size="xxs" variant="ghost" iconOnly aria-label={t('shell.commandTitle')} title={t('shell.commandShortcut')} onClick={onOpenCommands}><Icon name="search" size="sm" /></Button> : null}
      {onNew ? <Button size="xxs" variant="ghost" iconOnly aria-label={t('shell.newSession')} onClick={onNew}><Icon name="plus" size="sm" /></Button> : null}
      {onToggleFocus && active ? <Button size="xxs" variant="secondary" aria-label={t('shell.focus')} aria-pressed={focusedSurfaceId === active.id} title={t(focusedSurfaceId === active.id ? 'shell.restore' : 'shell.focus')} onClick={() => onToggleFocus(active.id)}><Icon name="copy" size="sm" /><span className="workspace-focus-label">{t(focusedSurfaceId === active.id ? 'shell.restore' : 'shell.focus')}</span></Button> : null}
    </div>
  </div>;
}

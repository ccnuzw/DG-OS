import { Dialog } from 'radix-ui';
import { PanelsTopLeft } from 'lucide-react';
import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { Button, DropdownMenu, Icon, Popover, type MenuItem } from '@dgos/ui';
import { useLocale } from '../i18n';
import type { WorkspaceSurface } from './model';
import type { WorkspaceAppEntry } from '../modules/moduleApi';
import { appUsable } from './useAppCatalog';

export type NavigationAction = { id: string; label: string; group: 'workspace' | 'apps' | 'settings'; icon: ReactNode; run: () => void; disabled?: boolean };

export function WorkspaceTools({ actions, open, onClose }: { actions: NavigationAction[]; open: boolean; onClose: () => void }) {
  const { t } = useLocale();
  const [query, setQuery] = useState('');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const origin = useRef<HTMLElement | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const id = useId();
  const filtered = actions.filter(action => `${action.label} ${action.id}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  const selected = filtered.findIndex(action => action.id === selectedId);
  const current = filtered[selected] ?? filtered.find(action => !action.disabled);
  const currentIndex = current ? filtered.indexOf(current) : -1;
  useEffect(() => { list.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest' }); }, [current?.id]);
  const execute = (action: NavigationAction) => { if (!action.disabled) { onClose(); action.run(); } };
  const move = (direction: number) => {
    for (let count = 1; count <= filtered.length; count++) {
      const next = (currentIndex + direction * count + filtered.length) % filtered.length;
      if (!filtered[next].disabled) { setSelectedId(filtered[next].id); break; }
    }
  };
  return <Dialog.Root open={open} onOpenChange={value => { if (!value) onClose(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-modal bg-overlay-scrim opacity-overlay" />
      <Dialog.Content aria-describedby={undefined} onOpenAutoFocus={() => { origin.current = document.activeElement as HTMLElement; setQuery(''); setSelectedId(null); }} onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { const target = origin.current; if (target?.isConnected && !target.closest('[hidden], [inert]') && !document.querySelector('.workspace-settings-dialog, .workspace-launcher-panel')) target.focus(); }); }} className="workspace-command">
        <Dialog.Title className="sr-only">{t('shell.commandTitle')}</Dialog.Title>
        <div className="workspace-command-search"><Icon name="search" /><input role="combobox" aria-expanded aria-controls={`${id}-list`} aria-activedescendant={current ? `${id}-${currentIndex}` : undefined} aria-label={t('shell.commandSearch')} value={query} onChange={event => { setQuery(event.target.value); setSelectedId(null); }} onKeyDown={event => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); move(event.key === 'ArrowDown' ? 1 : -1); }
          if (event.key === 'Home' || event.key === 'End') { event.preventDefault(); const enabled = filtered.filter(action => !action.disabled); setSelectedId((event.key === 'Home' ? enabled[0] : enabled[enabled.length - 1])?.id ?? null); }
          if (event.key === 'Enter' && current) { event.preventDefault(); execute(current); }
        }} placeholder={t('shell.commandSearch')} /><kbd>{t('shell.escapeKey')}</kbd></div>
        <div ref={list} id={`${id}-list`} role="listbox" aria-label={t('shell.commands')} className="workspace-command-results">
          {filtered.map((action, index) => <div key={action.id}><div className="workspace-command-group">{index === 0 || filtered[index - 1].group !== action.group ? t(action.group === 'apps' ? 'nav.modules' : action.group === 'settings' ? 'nav.settings' : 'shell.workspace') : null}</div><button id={`${id}-${index}`} type="button" role="option" tabIndex={-1} aria-selected={current?.id === action.id} aria-disabled={action.disabled} onMouseEnter={() => setSelectedId(action.id)} onClick={() => execute(action)} className="workspace-command-option">{action.icon}<span>{action.label}</span>{action.disabled ? <Icon name="stop" size="sm" /> : null}</button></div>)}
          {!filtered.length ? <p className="workspace-overlay-empty">{t('shell.noCommands')}</p> : null}
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

export function TabListPopover({ surfaces, activeId, onActivate, onClose, label }: {
  surfaces: WorkspaceSurface[]; activeId: string | null; onActivate: (id: string) => void; onClose: (id: string) => void; label: string;
}) {
  const { t } = useLocale();
  const [open, setOpen] = useState(false);
  const content = useRef<HTMLDivElement>(null);
  useEffect(() => { if (open) requestAnimationFrame(() => { const current = content.current?.querySelector<HTMLButtonElement>('[aria-current="true"]'); current?.focus(); current?.scrollIntoView({ block: 'nearest' }); }); }, [open]);
  return <Popover open={open} onOpenChange={setOpen} align="end" label={label} trigger={<Button size="xxs" variant="ghost" iconOnly className="text-status-info-text enabled:hover:bg-status-info-surface data-[state=open]:bg-status-info-surface" aria-label={label} title={label}><PanelsTopLeft className="size-3.5" aria-hidden="true" /></Button>} content={<div ref={content} className="workspace-tab-list" onKeyDown={event => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
    const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[data-tab-choice]'));
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length;
    event.preventDefault(); buttons[next]?.focus(); buttons[next]?.scrollIntoView({ block: 'nearest' });
  }}><p className="workspace-menu-title">{label} <span>{surfaces.length}</span></p>{surfaces.map(surface => {
    const title = surface.kind === 'ai' ? t('nav.workbench') : surface.title;
    return <div key={surface.id} className="workspace-tab-list-row"><button type="button" data-tab-choice aria-current={surface.id === activeId} title={title} onClick={() => { setOpen(false); onActivate(surface.id); }}><Icon name={surface.kind === 'ai' ? 'file' : 'folder'} size="sm" /><span>{title}</span>{surface.id === activeId ? <Icon name="check" size="sm" /> : null}</button><Button size="xxs" variant="ghost" iconOnly aria-label={t('shell.closeTab', { title })} onClick={() => { setOpen(false); onClose(surface.id); }}><Icon name="close" size="sm" /></Button></div>;
  })}</div>} />;
}

export function Launcher({ open, query, catalog, loading, error, onRetry, onManage, onQuery, onClose, onOpen, onPin, onUnpin, onMove, pinnedIds, activeDrag, onDragStart, onDragEnd, notice }: {
  open: boolean; query: string; catalog: WorkspaceAppEntry[]; loading: boolean; error: boolean; pinnedIds: string[]; activeDrag: string | null; notice: string;
  onRetry: () => void; onManage: () => void; onQuery: (value: string) => void; onClose: () => void; onOpen: (entry: WorkspaceAppEntry) => void; onPin: (entry: WorkspaceAppEntry) => void; onUnpin: (id: string) => void; onMove: (id: string, direction: -1 | 1) => void;
  onDragStart: (id: string) => void; onDragEnd: () => void;
}) {
  const { t } = useLocale();
  const origin = useRef<HTMLElement | null>(null);
  const hold = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const held = useRef(false);
  const clearHold = () => { clearTimeout(hold.current); hold.current = undefined; };
  useEffect(() => () => clearHold(), []);
  const visible = catalog.filter(entry => `${entry.displayName} ${entry.moduleId}`.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  const toggle = (entry: WorkspaceAppEntry) => pinnedIds.includes(entry.moduleId) ? onUnpin(entry.moduleId) : onPin(entry);
  return <Dialog.Root modal={false} open={open} onOpenChange={value => { if (!value && !activeDrag) onClose(); }}>
    {open ? <div className="workspace-launcher-layer" onPointerDown={event => { if (event.target === event.currentTarget && !activeDrag) onClose(); }}>
      <Dialog.Content aria-describedby={undefined} onOpenAutoFocus={() => { origin.current = document.activeElement as HTMLElement; }} onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { if (origin.current?.isConnected && !origin.current.closest('[hidden], [inert]') && !document.querySelector('.workspace-settings-dialog, .workspace-command')) origin.current.focus(); }); }} onInteractOutside={event => { if (activeDrag) event.preventDefault(); }} onEscapeKeyDown={event => { if (event.defaultPrevented) return; event.stopPropagation(); }} className="workspace-launcher-panel">
        <header className="workspace-launcher-head"><div><Dialog.Title>{t('nav.modules')}</Dialog.Title><p>{t('shell.workspace')}</p></div><div className="workspace-launcher-search"><Icon name="search" size="sm" /><input aria-label={t('shell.launcherSearch')} value={query} onChange={event => onQuery(event.target.value)} onKeyDown={event => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            const choices = event.currentTarget.closest('.workspace-launcher-panel')?.querySelectorAll<HTMLButtonElement>('.workspace-launcher-item');
            const target = event.key === 'ArrowDown' ? choices?.[0] : choices?.[choices.length - 1];
            if (target) { event.preventDefault(); target.focus(); target.scrollIntoView({ block: 'nearest' }); }
          } else if (event.key === 'Enter' && visible.length) { event.preventDefault(); onOpen(visible[0]); }
        }} placeholder={t('shell.launcherSearch')} /></div></header>
        {error ? <div role="alert" className="workspace-overlay-empty">{t('shell.catalogError')} <Button variant="secondary" onClick={onRetry}>{t('common.retry')}</Button></div> : <div className="workspace-launcher-grid" aria-busy={loading} onKeyDown={event => {
          if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key) || !(event.target instanceof HTMLElement) || !event.target.matches('.workspace-launcher-item')) return;
          const choices = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('.workspace-launcher-item'));
          const columns = getComputedStyle(event.currentTarget).gridTemplateColumns.split(' ').length;
          const offset = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : event.key === 'ArrowUp' ? -columns : columns;
          const index = choices.indexOf(event.target as HTMLButtonElement);
          const target = choices[(index + offset + choices.length) % choices.length];
          if (target) { event.preventDefault(); target.focus(); target.scrollIntoView({ block: 'nearest' }); }
        }}>{loading ? <p role="status">{t('common.loading')}</p> : visible.map(entry => {
          const usable = appUsable(entry);
          const pinned = pinnedIds.includes(entry.moduleId);
          const index = pinnedIds.indexOf(entry.moduleId);
          const items: MenuItem[] = [{ kind: 'item', id: 'pin', label: t(pinned ? 'shell.unpinApp' : 'shell.pin'), disabled: !usable && !pinned, onSelect: () => toggle(entry) }];
          if (pinned) items.push({ kind: 'item', id: 'up', label: t('shell.moveEarlier'), disabled: index === 0, onSelect: () => onMove(entry.moduleId, -1) }, { kind: 'item', id: 'down', label: t('shell.moveLater'), disabled: index === pinnedIds.length - 1, onSelect: () => onMove(entry.moduleId, 1) });
          return <div key={entry.moduleId} className="workspace-launcher-cell"><button type="button" draggable={usable} aria-disabled={!usable} title={`${entry.displayName}${usable ? '' : ` — ${t('shell.appUnavailable')}`}`} onClick={() => { if (held.current) { held.current = false; return; } onOpen(entry); }} onDragStart={event => { clearHold(); event.dataTransfer.setData('application/x-dgos-module', entry.moduleId); event.dataTransfer.effectAllowed = 'copy'; onDragStart(entry.moduleId); }} onDragEnd={onDragEnd} onPointerDown={event => { held.current = false; if (event.pointerType === 'touch' && usable) hold.current = setTimeout(() => { held.current = true; toggle(entry); }, 550); }} onPointerUp={clearHold} onPointerCancel={clearHold} onPointerMove={clearHold} className="workspace-launcher-item" data-blocked={!usable}>
            <span className="workspace-launcher-icon"><Icon name={entry.layout === 'full-bleed' ? 'palette' : 'code'} size="lg" />{usable ? <i aria-hidden="true" /> : null}</span><span className="workspace-launcher-name">{entry.displayName}</span>{pinned ? <span className="sr-only">{t('shell.pinned')}</span> : null}
          </button><DropdownMenu label={entry.displayName} items={items} trigger={<Button size="xxs" variant="ghost" iconOnly aria-label={t('shell.appActions', { title: entry.displayName })}><Icon name="more" size="sm" /></Button>} /></div>;
        })}{!loading && !visible.length ? <p className="workspace-overlay-empty">{t(catalog.length ? 'shell.noApps' : 'shell.catalogEmpty')}</p> : null}</div>}
        <footer><p role="status" aria-live="polite">{notice}</p><Button variant="ghost" onClick={() => { onClose(); onManage(); }}>{t('shell.openManagement')}</Button></footer>
      </Dialog.Content>
    </div> : null}
  </Dialog.Root>;
}

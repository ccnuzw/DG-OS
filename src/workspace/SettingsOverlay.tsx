import { createContext, useContext, useEffect, useRef, type ReactNode } from 'react';
import { UNSAFE_NavigationContext, useLocation } from 'react-router-dom';
import { Dialog } from 'radix-ui';
import { Button } from '@dgos/ui';
import { useLocale } from '../i18n';
import type { WorkspaceController } from './controller';
import { SurfaceScope, useSurfaceCloseGuard } from './SurfaceContext';
import { intentForPath } from './routes';

interface CategoryBoundary { id: string; contains: (pathname: string) => boolean }
const SettingsContext = createContext<{ controller: WorkspaceController; category: { current: CategoryBoundary | null } } | null>(null);

// Window is popstate's target: listener registration order wins even for capture listeners.
// Install before BrowserRouter mounts, otherwise its listener can unmount the guard before it runs.
let interceptHistory: ((event: PopStateEvent) => void) | null = null;
window.addEventListener('popstate', event => interceptHistory?.(event), true);

// Kept pages contribute to overlay closure, but only an unmounted category guards category changes.
export function SettingsCategoryScope({ id, active, keptAlive, contains, children }: CategoryBoundary & { active: boolean; keptAlive: boolean; children: ReactNode }) {
  const settings = useContext(SettingsContext);
  useSurfaceCloseGuard(async () => !settings || await settings.controller.canLeave(id));
  useEffect(() => {
    if (!settings || !active || keptAlive) return;
    const boundary = { id, contains };
    settings.category.current = boundary;
    return () => { if (settings.category.current === boundary) settings.category.current = null; };
  }, [settings, id, active, keptAlive, contains]);
  return settings ? <SurfaceScope id={id} active={active} controller={settings.controller}>{children}</SurfaceScope> : children;
}

export function SettingsOverlay({ controller, onClose, children }: { controller: WorkspaceController; onClose: () => Promise<void>; children: ReactNode }) {
  const { t } = useLocale();
  const { navigator } = useContext(UNSAFE_NavigationContext);
  const location = useLocation();
  const category = useRef<CategoryBoundary | null>(null);
  const origin = useRef<HTMLElement | null>(null);
  useEffect(() => {
    let pending = false;
    let disposed = false;
    const canNavigate = (pathname: string) => {
      if (intentForPath(pathname)?.kind !== 'overlay') return controller.canLeave('settings');
      const boundary = category.current;
      return boundary && !boundary.contains(pathname) ? controller.canLeave(boundary.id) : Promise.resolve(true);
    };
    const run = async (pathname: string, action: () => void) => {
      if (pending) return;
      if (intentForPath(pathname)?.kind === 'overlay' && (!category.current || category.current.contains(pathname))) { action(); return; }
      pending = true;
      try { if (await canNavigate(pathname) && !disposed) action(); } finally { pending = false; }
    };
    const push = navigator.push;
    const replace = navigator.replace;
    const go = navigator.go;
    navigator.push = (...args) => { const to = args[0]; void run(typeof to === 'string' ? to.split(/[?#]/)[0] : to.pathname ?? location.pathname, () => push.apply(navigator, args)); };
    navigator.replace = (...args) => { const to = args[0]; void run(typeof to === 'string' ? to.split(/[?#]/)[0] : to.pathname ?? location.pathname, () => replace.apply(navigator, args)); };
    // Browser traversal is guarded by its popstate destination; MemoryRouter has no popstate.
    const browserHistory = typeof window.history.state?.idx === 'number' && (window.history.state?.key ?? 'default') === location.key;
    navigator.go = (...args) => { if (browserHistory) go.apply(navigator, args); else void run('', () => go.apply(navigator, args)); };
    // Browser back/forward bypass navigator.go. Restore the current history slot before asking,
    // then replay the exact delta only after consent. The background DOM never changes on refusal.
    const index = window.history.state?.idx as number | undefined;
    let restoreDelta: number | null = null;
    let replay = false;
    let destination = '';
    const pop = (event: PopStateEvent) => {
      if (replay) { replay = false; return; }
      if (restoreDelta !== null) {
        event.stopImmediatePropagation();
        const delta = restoreDelta;
        restoreDelta = null;
        void run(destination, () => { replay = true; window.history.go(delta); });
        return;
      }
      const next = event.state?.idx as number | undefined;
      if (index === undefined || next === undefined || next === index) return;
      event.stopImmediatePropagation();
      destination = window.location.pathname;
      restoreDelta = next - index;
      window.history.go(-restoreDelta);
    };
    interceptHistory = pop;
    return () => { disposed = true; navigator.push = push; navigator.replace = replace; navigator.go = go; if (interceptHistory === pop) interceptHistory = null; };
  }, [controller, navigator, location.key, location.pathname]);
  return <Dialog.Root open onOpenChange={open => { if (!open) void onClose(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-modal bg-overlay-scrim opacity-overlay" />
      <Dialog.Content aria-describedby={undefined} onInteractOutside={event => event.preventDefault()}
        onOpenAutoFocus={() => { origin.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; }}
        onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { const target = origin.current?.isConnected && !origin.current.closest('[hidden], [inert]') ? origin.current : document.querySelector<HTMLElement>('[data-workspace-tab][aria-selected="true"], a[href="/settings"], a[href="/"]'); target?.focus(); }); }}
        className="workspace-settings-dialog">
        <header className="workspace-settings-header"><Dialog.Title>{t('settings.title')}</Dialog.Title><Button variant="ghost" size="sm" onClick={() => void onClose()}>{t('shell.closeSettings')}</Button></header>
        <div className="min-h-0 flex-1 overflow-clip"><SettingsContext.Provider value={{ controller, category }}><SurfaceScope id="settings" active controller={controller}>{children}</SurfaceScope></SettingsContext.Provider></div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

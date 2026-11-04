import { useContext, useEffect, useRef, type ReactNode } from 'react';
import { UNSAFE_NavigationContext } from 'react-router-dom';
import { Dialog } from 'radix-ui';
import { Button } from '@dgos/ui';
import { useLocale } from '../i18n';
import type { WorkspaceController } from './controller';
import { SurfaceScope } from './SurfaceContext';
import { intentForPath } from './routes';

export function SettingsOverlay({ controller, onClose, children }: { controller: WorkspaceController; onClose: () => Promise<void>; children: ReactNode }) {
  const { t } = useLocale();
  const { navigator } = useContext(UNSAFE_NavigationContext);
  const origin = useRef<HTMLElement | null>(null);
  useEffect(() => {
    let pending = false;
    let disposed = false;
    const run = async (action: () => void) => {
      if (pending) return;
      pending = true;
      try { if (await controller.canLeave('settings') && !disposed) action(); } finally { pending = false; }
    };
    const push = navigator.push;
    const replace = navigator.replace;
    const go = navigator.go;
    const isCategory = (to: Parameters<typeof push>[0]) => intentForPath(typeof to === 'string' ? to.split(/[?#]/)[0] : to.pathname ?? '')?.kind === 'overlay';
    navigator.push = (...args) => { if (isCategory(args[0])) push.apply(navigator, args); else void run(() => push.apply(navigator, args)); };
    navigator.replace = (...args) => { if (isCategory(args[0])) replace.apply(navigator, args); else void run(() => replace.apply(navigator, args)); };
    navigator.go = (...args) => { void run(() => go.apply(navigator, args)); };
    // Browser back/forward bypass navigator.go. Restore the current history slot before asking,
    // then replay the exact delta only after consent. The background DOM never changes on refusal.
    const index = window.history.state?.idx as number | undefined;
    let restoreDelta: number | null = null;
    let replay = false;
    const pop = (event: PopStateEvent) => {
      if (replay) { replay = false; return; }
      if (restoreDelta !== null) {
        event.stopImmediatePropagation();
        const delta = restoreDelta;
        restoreDelta = null;
        void run(() => { replay = true; window.history.go(delta); });
        return;
      }
      const next = event.state?.idx as number | undefined;
      if (index === undefined || next === undefined || next === index) return;
      event.stopImmediatePropagation();
      restoreDelta = next - index;
      window.history.go(-restoreDelta);
    };
    window.addEventListener('popstate', pop, true);
    return () => { disposed = true; navigator.push = push; navigator.replace = replace; navigator.go = go; window.removeEventListener('popstate', pop, true); };
  }, [controller, navigator]);
  return <Dialog.Root open onOpenChange={open => { if (!open) void onClose(); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-modal bg-overlay opacity-overlay" />
      <Dialog.Content aria-describedby={undefined} onInteractOutside={event => event.preventDefault()}
        onOpenAutoFocus={() => { origin.current = document.activeElement instanceof HTMLElement ? document.activeElement : null; }}
        onCloseAutoFocus={event => { event.preventDefault(); requestAnimationFrame(() => { const target = origin.current?.isConnected && !origin.current.closest('[hidden], [inert]') ? origin.current : document.querySelector<HTMLElement>('[data-workspace-tab][aria-selected="true"], a[href="/settings"], a[href="/"]'); target?.focus(); }); }}
        className="workspace-settings-dialog">
        <header className="workspace-settings-header"><Dialog.Title>{t('settings.title')}</Dialog.Title><Button variant="ghost" size="sm" onClick={() => void onClose()}>{t('shell.closeSettings')}</Button></header>
        <div className="min-h-0 flex-1 overflow-hidden"><SurfaceScope id="settings" active controller={controller}>{children}</SurfaceScope></div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

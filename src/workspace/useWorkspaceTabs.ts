import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';
import { useLocale } from '../i18n';
import type { WorkspaceSurface } from './model';
import { canonicalPath, intentForPath, pathForSurface } from './routes';
import { createWorkspaceController, useWorkspaceState, type WorkspaceController } from './controller';

export interface WorkspaceTabs {
  surfaces: WorkspaceSurface[];
  activeSurfaceId: string | null;
  onActivate: (surfaceId: string) => void;
  onClose: (surfaceId: string) => void;
}
export interface WorkspaceNavigation extends WorkspaceTabs {
  controller: WorkspaceController;
  backgroundSurfaceId: string | null;
  settingsOpen: boolean;
  closeSettings: () => Promise<void>;
}
export function useWorkspaceTabs(supplied?: WorkspaceController): WorkspaceNavigation {
  const [local] = useState(createWorkspaceController);
  const controller = supplied ?? local;
  const state = useWorkspaceState(controller);
  const location = useLocation();
  const latestLocation = useRef(location);
  const navigate = useNavigate();
  const { t } = useLocale();
  const intent = intentForPath(location.pathname);
  const last = useRef({ pathname: '/', key: '', surfaceId: null as string | null });
  const background = useRef<{ surfaceId: string | null; pathname: string; key: string } | null>(null);
  const wasSettings = useRef(false);
  const [backgroundId, setBackgroundId] = useState<string | null>(null);
  const settingsOpen = intent?.kind === 'overlay';
  useEffect(() => {
    const canonical = canonicalPath(location.pathname);
    if (canonical !== location.pathname) navigate(canonical, { replace: true });
  }, [location.pathname, navigate]);
  // The controller is the only active-surface owner; URL changes enqueue its projection before paint.
  useLayoutEffect(() => {
    latestLocation.current = location;
    if (settingsOpen) {
      if (!wasSettings.current) { background.current = last.current.key ? { ...last.current, surfaceId: controller.getSnapshot().activeSurfaceId } : null; setBackgroundId(background.current?.surfaceId ?? null); }
    } else {
      background.current = null;
      if (intent?.kind === 'tab') {
        const surface: WorkspaceSurface = intent.surfaceKind === 'ai' ? { kind: 'ai', id: 'ai' }
          : intent.surfaceKind === 'resource' ? { kind: 'resource', id: intent.surfaceId as '/skills' | '/mcp', title: t(intent.titleKey) }
          : { kind: 'task', id: intent.surfaceId as 'single-task' | 'cli-agent', title: t(intent.titleKey) };
        controller.open(surface);
      } else if (intent?.kind !== 'module') controller.dispatch({ type: 'surface/deactivate' });
      last.current = { pathname: location.pathname, key: location.key, surfaceId: controller.getSnapshot().activeSurfaceId };
    }
    wasSettings.current = settingsOpen;
  }, [controller, location, settingsOpen]);
  const surfaces = state.surfaces.map(surface => {
    if (surface.kind === 'app' || surface.kind === 'ai') return surface;
    const titleKey = surface.kind === 'resource' ? (surface.id === '/mcp' ? 'nav.mcp' : 'nav.skills') : (surface.id === 'single-task' ? 'workbench.single' : 'workbench.agent');
    return { ...surface, title: t(titleKey) };
  });
  return {
    surfaces,
    activeSurfaceId: settingsOpen ? null : state.activeSurfaceId,
    controller,
    backgroundSurfaceId: settingsOpen ? backgroundId : state.activeSurfaceId,
    settingsOpen,
    onActivate(id) { const path = pathForSurface(id, state.surfaces); if (path && path !== location.pathname) navigate(path); },
    onClose(id) {
      const source = location;
      const sourceFocus = document.activeElement;
      const activeSurfaceId = controller.getSnapshot().activeSurfaceId;
      void controller.requestClose(id).then(closed => {
        // Consent can outlive this route. Closing the old surface must preserve the user's newer destination and focus.
        if (!closed || settingsOpen || latestLocation.current !== source) return;
        const current = controller.getSnapshot();
        const path = current.activeSurfaceId ? pathForSurface(current.activeSurfaceId, current.surfaces) ?? '/' : '/';
        if (current.activeSurfaceId !== activeSurfaceId) navigate(path);
        const closeFocus = document.activeElement;
        if (closeFocus !== sourceFocus && closeFocus !== document.body && !(closeFocus instanceof HTMLElement && closeFocus.closest('[role="dialog"]'))) return;
        requestAnimationFrame(() => {
          if (latestLocation.current.pathname !== path || controller.getSnapshot().activeSurfaceId !== current.activeSurfaceId) return;
          if (document.activeElement !== closeFocus && document.activeElement !== sourceFocus && document.activeElement !== document.body) return;
          const next = Array.from(document.querySelectorAll<HTMLButtonElement>('[data-workspace-tab]')).find(button => button.dataset.workspaceTab === current.activeSurfaceId);
          (next ?? document.querySelector<HTMLAnchorElement>('a[href="/"]'))?.focus();
        });
      });
    },
    async closeSettings() {
      const source = background.current;
      if (source) navigate(-1);
      else navigate('/', { replace: true });
    },
  };
}

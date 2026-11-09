import { lazy, Suspense, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Routes, Route, useLocation, useNavigate } from "react-router-dom";
import McpConnectionNotice from "./components/McpConnectionNotice";
import ModuleHostBootstrap from "./modules/ModuleHostBootstrap";
import ModuleSurface from "./modules/ModuleSurface";
import { Button, ErrorPage } from "@dgos/ui";
import { isHostSettingsPath } from "./modules/hostRoutes";
import { ROUTES } from "./appRoutes";
import { AppShell } from "./workspace/AppShell";
import { useWorkspaceTabs } from "./workspace/useWorkspaceTabs";
import { useWorkspaceState } from './workspace/controller';
import { SurfaceScope } from './workspace/SurfaceContext';
import { SettingsOverlay } from './workspace/SettingsOverlay';
import { appUsable, useAppCatalog } from './workspace/useAppCatalog';
import { appPath, intentForPath } from './workspace/routes';
import { prepareChatClose } from './workspace/closeCoordinator';
import AccountMenu from "./accounts/AccountMenu";
import { useLocale } from './i18n';
import { Launcher, WorkspaceTools } from './workspace/WorkspaceTools';
import { useAppActions } from './workspace/useAppActions';
import { buildAppNavigation, buildBuiltinNavigation, buildCommandActions } from './workspace/navigation';
import './workspace/workspace.css';
import Desktop from './pages/Desktop';
import './modules/provider-console.css';
import { useDesktopPreferences } from './workspace/desktopPreferences';
import { useLayoutBudget } from './ui/layout/useLayoutBudget';
import { shellMinimumWidth } from './ui/layout/layoutBudgets';
const SkillsManagement = lazy(() => import("./pages/SkillsManagement"));
const Workbench = lazy(() => import("./components/chat/ChatWorkbench"));
const SingleTask = lazy(() => import('./pages/workbench/SingleTaskWorkbench'));
const Agent = lazy(() => import('./pages/workbench/AgentWorkbench'));
const McpManagement = lazy(() => import("./pages/McpManagement"));
const SettingsShell = lazy(() => import("./pages/SettingsShell"));

// Router matching decodes the path, so `/%73ettings` reaches this route. The raw location path is
// what the Rust gate sees (D037), so the page guards on it and never shows Settings where Rust denies.
function SettingsPage() {
  const { t } = useLocale();
  const { pathname } = useLocation();
  if (!isHostSettingsPath(pathname)) return <div className="p-6">{t('system.notFound')}</div>;
  return <SettingsShell />;
}


// AC19: a path that matches no route shows the shared error page, not a blank content area.
export function NotFoundRoute() {
  const navigate = useNavigate();
  return <ErrorPage kind="not-found" onHome={() => navigate("/")} />;
}



function App() {
  const { t } = useLocale();
  const location = useLocation();
  const navigate = useNavigate();
  const tabs = useWorkspaceTabs();
  const controller = tabs.controller;
  const state = useWorkspaceState(controller);
  const statuses = useSyncExternalStore(controller.subscribeSurfaceStatuses, controller.getSurfaceStatuses, controller.getSurfaceStatuses);
  const { catalog, error: catalogError, refresh } = useAppCatalog(location.pathname);
  const desktop = useDesktopPreferences();
  const userCollapsed = desktop.values.sidebarCollapsed;
  useEffect(() => { void useDesktopPreferences.getState().load(); }, []);
  useEffect(() => { if (desktop.loaded) controller.dispatch({ type: 'preferences/hydrate', pinnedModuleIds: desktop.values.pinnedModuleIds }); }, [desktop.loaded, desktop.values.pinnedModuleIds, controller]);
  const savePins = () => useDesktopPreferences.getState().patch({ pinnedModuleIds: controller.getSnapshot().pinnedModuleIds });
  const { root: shellRoot, narrow } = useLayoutBudget<HTMLDivElement>(shellMinimumWidth);
  const [toolsOpen, setToolsOpen] = useState(false);
  const intent = intentForPath(location.pathname);
  const entries = catalog?.entries ?? [];
  const { go, openLauncher, closeLauncher, activateSurface, openApp, pinApp, unpin, movePin, dropPin, launcherNotice } = useAppActions(tabs, entries, desktop.loaded, savePins);
  const toggleFocus = (id: string) => {
    if (!state.focusedSurfaceId) controller.setLayout({ ...controller.getLayout(), sidebar: !userCollapsed });
    controller.toggleFocus(id);
  };
  useEffect(() => controller.registerGuard('ai', prepareChatClose), [controller]);
  const openedRoute = useRef<string | null>(null);
  useEffect(() => {
    const old = /^\/settings\/modules\/([^/]+)$/.exec(location.pathname);
    if (old) { try { navigate(appPath(decodeURIComponent(old[1])), { replace: true }); } catch { navigate(ROUTES.modules, { replace: true }); } return; }
    if (intent?.kind !== 'module' || !catalog) return;
    const entry = catalog.entries.find(value => value.moduleId === intent.moduleId);
    const routeKey = JSON.stringify([location.key, location.pathname]);
    if (!entry || openedRoute.current === routeKey) return;
    const outcome = controller.dispatch({ type: 'app/open', moduleId: entry.moduleId, documentId: entry.viewCapability === 'single' ? null : intent.documentId, title: entry.displayName, usable: entry.enabled && entry.permissionState === 'granted', runtimeReady: entry.runtimeState === 'ready' && !entry.crashed });
    if (outcome.result === 'created' || outcome.result === 'focused') openedRoute.current = routeKey;
  }, [location.key, location.pathname, catalog, navigate, controller]);
  useEffect(() => {
    if (!catalog || !desktop.loaded) return;
    for (const moduleId of controller.getSnapshot().pinnedModuleIds) {
      if (!catalog.entries.some(entry => entry.moduleId === moduleId)) { controller.dispatch({ type: 'app/unpin', moduleId }); savePins(); }
    }
  }, [catalog, controller, desktop.loaded, desktop.values.pinnedModuleIds]);
  useEffect(() => { if (state.launcher.open) void refresh(); }, [state.launcher.open, refresh]);
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        if (document.querySelector('[data-modal-kind], [role="alertdialog"], [data-state="open"][role="menu"]')) return;
        event.preventDefault(); closeLauncher(); setToolsOpen(value => !value); return;
      }
      if (event.key === 'Escape' && state.focusedSurfaceId && !state.launcher.open && !tabs.settingsOpen && !document.querySelector('[role="dialog"], [role="listbox"], [data-radix-popper-content-wrapper]')) { controller.toggleFocus(state.focusedSurfaceId); event.preventDefault(); }
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [state.focusedSurfaceId, state.launcher.open, tabs.settingsOpen, controller]);
  const builtin = buildBuiltinNavigation(t, go);
  const apps = buildAppNavigation(entries, state.pinnedModuleIds, openApp);
  const actions = buildCommandActions(t, { builtin, apps, surfaces: tabs.surfaces, activeSurfaceId: state.activeSurfaceId, focusedSurfaceId: state.focusedSurfaceId, openLauncher, activateSurface, toggleFocus });
  const shownId = tabs.backgroundSurfaceId;
  const shown = state.surfaces.find(surface => surface.id === shownId);
  const fullBleed = shown?.kind === 'app' && entries.find(entry => entry.moduleId === shown.moduleId)?.layout === 'full-bleed';
  const focused = state.focusedSurfaceId !== null && state.focusedSurfaceId === shownId;
  return <div ref={shellRoot} className="flex h-screen w-full min-w-0 flex-col">
    <ModuleHostBootstrap />
    <AppShell navEntries={builtin.primary} navGroups={builtin.groups} pinnedEntries={apps.pinnedEntries}
      collapsed={userCollapsed || narrow} autoCollapsed={narrow} onToggleCollapsed={() => desktop.patch({ sidebarCollapsed: !userCollapsed })}
      onOpenLauncher={openLauncher} launcherOpen={state.launcher.open} onPinModule={dropPin} onUnpinModule={unpin} onMovePin={movePin} onDragModule={moduleId => controller.dispatch(moduleId ? { type: 'launcher/drag-start', moduleId } : { type: 'launcher/drag-end' })}
      draggingModuleId={state.launcher.dragModuleId} railHidden={focused || fullBleed} railFooter={<AccountMenu collapsed />} settingsEntry={builtin.settings}
      tabs={{ ...tabs, focusedSurfaceId: state.focusedSurfaceId, surfaceStatuses: statuses, runningModuleIds: entries.filter(entry => appUsable(entry)).map(entry => entry.moduleId), onToggleFocus: toggleFocus, onOpenLauncher: openLauncher, onOpenCommands: () => setToolsOpen(true) }}>
      <McpConnectionNotice />
      {desktop.error ? <div role="alert" className="flex flex-wrap items-center gap-2 p-2"><span>{t(desktop.error === 'conflict' ? 'home.preferencesConflict' : 'home.preferencesFailed')}</span><Button variant="secondary" disabled={desktop.busy} onClick={() => void desktop.load()}>{t('home.reloadPreferences')}</Button></div> : null}
      <div className="app-content h-full min-h-0 overflow-hidden">
        <div hidden={shownId !== null || (!tabs.settingsOpen && intent?.kind !== 'bare')} inert={shownId !== null || tabs.settingsOpen || state.launcher.open || toolsOpen} className="h-full overflow-auto"><Desktop entries={entries} onOpenApps={openLauncher} onOpenApp={openApp} />{catalogError ? <p role="alert">{t('shell.catalogError')} <Button variant="secondary" onClick={() => void refresh()}>{t('common.retry')}</Button></p> : null}</div>
        {state.surfaces.map(surface => {
          const active = surface.id === shownId;
          return <div key={surface.id} hidden={!active} inert={!active || tabs.settingsOpen || state.launcher.open || toolsOpen} className="h-full min-h-0 overflow-auto" data-workspace-surface={surface.id} data-app-layout={surface.kind === 'app' ? entries.find(entry => entry.moduleId === surface.moduleId)?.layout : undefined}>
            <SurfaceScope id={surface.id} visible={active} active={active && !tabs.settingsOpen && !state.launcher.open && !toolsOpen} controller={controller}>
              <Suspense fallback={<div role="status">{t('common.loading')}</div>}>
                {surface.kind === 'ai' ? <Workbench /> : surface.kind === 'resource' ? surface.id === '/skills' ? <SkillsManagement /> : <McpManagement /> : surface.kind === 'task' ? surface.id === 'single-task' ? <SingleTask /> : <Agent /> : <ModuleSurface surface={surface} available={entries.some(entry => entry.moduleId === surface.moduleId && appUsable(entry))} />}
              </Suspense>
            </SurfaceScope>
          </div>;
        })}
        {!tabs.settingsOpen && intent === null ? <NotFoundRoute /> : null}
        {!tabs.settingsOpen && intent?.kind === 'module' && !shown ? <div className="p-6"><p role="status">{t('shell.unavailable')}</p><Button onClick={builtin.settings.run}>{t('shell.openManagement')}</Button></div> : null}
      </div>
      <Launcher open={state.launcher.open} query={state.launcher.query} catalog={entries} loading={!catalog && !catalogError} error={catalogError} notice={launcherNotice} onRetry={() => void refresh()} onManage={() => go(ROUTES.modules)} onQuery={query => controller.dispatch({ type: 'launcher/search', query })} onClose={closeLauncher} onOpen={openApp} onPin={pinApp} onUnpin={unpin} onMove={movePin} pinnedIds={state.pinnedModuleIds} activeDrag={state.launcher.dragModuleId} onDragStart={moduleId => controller.dispatch({ type: 'launcher/drag-start', moduleId })} onDragEnd={() => controller.dispatch({ type: 'launcher/drag-end' })} />
      {tabs.settingsOpen ? <SettingsOverlay controller={controller} onClose={tabs.closeSettings}><Suspense fallback={<div role="status">{t('common.loading')}</div>}>{isHostSettingsPath(location.pathname) ? <Routes><Route path="/settings/*" caseSensitive element={<SettingsPage />} /></Routes> : null}</Suspense></SettingsOverlay> : null}
      <WorkspaceTools actions={actions} open={toolsOpen} onClose={() => setToolsOpen(false)} />
    </AppShell>
  </div>;
}

export default App;

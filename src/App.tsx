import { lazy, Suspense, useEffect, useState, useSyncExternalStore } from "react";
import { Routes, Route, useLocation, useNavigate } from "react-router-dom";
import McpConnectionNotice from "./components/McpConnectionNotice";
import ModuleHostBootstrap from "./modules/ModuleHostBootstrap";
import { ModuleSlot, useModuleInstance } from "./modules/ModuleSlots";
import { Button, ErrorPage, Icon, Modal } from "@dgos/ui";
import { isHostSettingsPath } from "./modules/hostRoutes";
import { ROUTES } from "./appRoutes";
import { AppShell, type NavEntry } from "./workspace/AppShell";
import { useWorkspaceTabs } from "./workspace/useWorkspaceTabs";
import { useWorkspaceState } from './workspace/controller';
import { SurfaceScope } from './workspace/SurfaceContext';
import { SettingsOverlay } from './workspace/SettingsOverlay';
import { useAppCatalog } from './workspace/useAppCatalog';
import { appPath, intentForPath } from './workspace/routes';
import { prepareChatClose } from './workspace/closeCoordinator';
import type { WorkspaceSurface } from './workspace/model';
import { moduleRuntimeHost, type ModuleDocumentView } from './modules/moduleRuntimeHost';
import { useSurfaceCloseGuard, useSurfaceStatus } from './workspace/SurfaceContext';
import { confirmDiscard } from './workspace/closeCoordinator';
import { ChatGlyph, DesktopGlyph, SettingsGlyph, SkillGlyph } from "./workspace/icons";
import AccountMenu from "./accounts/AccountMenu";
import { useLocale } from './i18n';
import { appUsable, Launcher, WorkspaceTools, type NavigationAction } from './workspace/WorkspaceTools';
import { useChatStore } from './stores/chatStore';
import type { WorkspaceAppEntry } from './modules/moduleApi';
import './workspace/workspace.css';
import Desktop from './pages/Desktop';
import './modules/provider-console.css';
import { useDesktopPreferences } from './workspace/desktopPreferences';
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


function AppView({ surface, available }: { surface: Extract<WorkspaceSurface, {kind: 'app'}>; available: boolean }) {
  const instance = useModuleInstance(surface.moduleId);
  const [view, setView] = useState<ModuleDocumentView | null>(null);
  const [closePrompt, setClosePrompt] = useState<{ resolve: (value: boolean) => void; runtimeId: string } | null>(null);
  const { t } = useLocale();
  const navigate = useNavigate();
  const [dirty, setDirty] = useState(false);
  useSurfaceStatus({ dirty });
  useEffect(() => {
    if (!instance || !view || !available) { setDirty(false); return; }
    let current = true;
    const update = () => { void moduleRuntimeHost.prepareViewClose(instance.runtimeId, surface.viewId).then(status => { if (current) setDirty(status.dirty); }).catch(() => {}); };
    update();
    const timer = window.setInterval(update, 500);
    return () => { current = false; clearInterval(timer); };
  }, [instance?.runtimeId, view, available, surface.viewId]);
  useEffect(() => {
    let active = true;
    setView(null);
    if (!instance || !available || instance.crashed) return;
    void moduleRuntimeHost.openView(instance.runtimeId, surface.viewId, surface.documentId).then(value => {
      if (active) setView(value);
    }).catch(() => { if (active) setView(null); });
    return () => { active = false; moduleRuntimeHost.closeView(instance.runtimeId, surface.viewId); };
  }, [instance?.runtimeId, instance?.crashed, available, surface.viewId, surface.documentId]);
  useSurfaceCloseGuard(async () => {
    if (!instance || !available) return true;
    const status = await moduleRuntimeHost.prepareViewClose(instance.runtimeId, surface.viewId);
    if (status.blocked) return false;
    if (!status.dirty) return true;
    if (!status.canSave) return await confirmDiscard();
    return new Promise<boolean>(resolve => setClosePrompt({ resolve, runtimeId: instance.runtimeId }));
  });
  if (!available || !instance || instance.crashed || !view) return <div className="p-6"><p role="status">{t('shell.unavailable')}</p><Button onClick={() => navigate(ROUTES.modules)}>{t('shell.openManagement')}</Button></div>;
  return <><ModuleSlot instance={view} />{closePrompt ? <Modal open title={t('shell.unsavedTitle')} description={t('shell.unsavedSaveConfirm')}
    onOpenChange={open => { if (!open) { closePrompt.resolve(false); setClosePrompt(null); } }}
    footer={<>
      <Button variant="secondary" onClick={() => { closePrompt.resolve(false); setClosePrompt(null); }}>{t('common.cancel')}</Button>
      <Button variant="danger" onClick={() => { closePrompt.resolve(true); setClosePrompt(null); }}>{t('shell.discard')}</Button>
      <Button onClick={() => { void moduleRuntimeHost.saveView(closePrompt.runtimeId, surface.viewId).then(saved => { if (saved) { closePrompt.resolve(true); setClosePrompt(null); } }); }}>{t('shell.saveAndClose')}</Button>
    </>} /> : null}</>;
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
  const [narrow, setNarrow] = useState(() => window.matchMedia?.('(max-width: 1119px)').matches ?? false);
  const [toolsOpen, setToolsOpen] = useState(false);
  const [launcherNotice, setLauncherNotice] = useState('');
  const chatRunning = useChatStore(value => Object.keys(value.generations).length > 0);
  const chatDirty = useChatStore(value => Object.values(value.sessionDrafts).some(Boolean) || Object.values(value.sessionTargetDrafts).some(Boolean));
  const intent = intentForPath(location.pathname);
  const entries = catalog?.entries ?? [];
  const go = (path: string) => { controller.dispatch({ type: 'launcher/close' }); navigate(path); };
  const openLauncher = () => { setLauncherNotice(''); controller.dispatch({ type: 'launcher/open' }); };
  const closeLauncher = () => { controller.dispatch({ type: 'launcher/close' }); };
  const openApp = (entry: WorkspaceAppEntry) => {
    if (!appUsable(entry)) { setLauncherNotice(t('shell.unavailable')); return; }
    const existing = controller.getSnapshot().surfaces.find(surface => surface.kind === 'app' && surface.moduleId === entry.moduleId);
    if (existing) { closeLauncher(); tabs.onActivate(existing.id); } else go(appPath(entry.moduleId));
  };
  const pinApp = (entry: WorkspaceAppEntry) => {
    if (!desktop.loaded) { setLauncherNotice(t('common.loading')); return; }
    if (!appUsable(entry)) { setLauncherNotice(t('shell.unavailable')); return; }
    const outcome = controller.dispatch({ type: 'app/pin', moduleId: entry.moduleId });
    savePins();
    setLauncherNotice(t(outcome.result === 'limit' ? 'shell.pinLimit' : outcome.result === 'already-pinned' ? 'shell.alreadyPinned' : 'shell.pinned'));
  };
  const unpin = (moduleId: string) => { if (!desktop.loaded) return; controller.dispatch({ type: 'app/unpin', moduleId }); savePins(); setLauncherNotice(t('shell.unpinned')); };
  const movePin = (moduleId: string, direction: -1 | 1) => { if (!desktop.loaded) return; controller.dispatch({ type: 'app/move-pin', moduleId, targetIndex: controller.getSnapshot().pinnedModuleIds.indexOf(moduleId) + direction }); savePins(); };
  const dropPin = (moduleId: string, targetIndex?: number) => {
    if (!desktop.loaded) return;
    const entry = entries.find(value => value.moduleId === moduleId);
    if (entry && appUsable(entry)) {
      pinApp(entry);
      if (targetIndex !== undefined) controller.dispatch({ type: 'app/move-pin', moduleId, targetIndex });
      savePins();
    }
    controller.dispatch({ type: 'launcher/drag-end' });
  };
  const toggleFocus = (id: string) => {
    if (!state.focusedSurfaceId) controller.setLayout({ ...controller.getLayout(), sidebar: !userCollapsed });
    controller.toggleFocus(id);
  };
  useEffect(() => {
    const media = window.matchMedia?.('(max-width: 1119px)');
    const update = () => setNarrow(media?.matches ?? false);
    media?.addEventListener('change', update);
    return () => media?.removeEventListener('change', update);
  }, []);
  useEffect(() => controller.registerGuard('ai', prepareChatClose), [controller]);
  useEffect(() => {
    const old = /^\/settings\/modules\/([^/]+)$/.exec(location.pathname);
    if (old) { try { navigate(appPath(decodeURIComponent(old[1])), { replace: true }); } catch { navigate(ROUTES.modules, { replace: true }); } return; }
    if (intent?.kind !== 'module' || !catalog) return;
    const entry = catalog.entries.find(value => value.moduleId === intent.moduleId);
    if (entry) controller.dispatch({ type: 'app/open', moduleId: entry.moduleId, documentId: entry.viewCapability === 'single' ? null : intent.documentId, title: entry.displayName, usable: entry.enabled && entry.permissionState === 'granted', runtimeReady: entry.runtimeState === 'ready' && !entry.crashed });
  }, [location.pathname, catalog, navigate, controller]);
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
  const primary: NavEntry[] = [
    { id: 'desktop', label: t('nav.home'), to: ROUTES.home, end: true, icon: <DesktopGlyph className="size-4" />, run: () => go(ROUTES.home) },
    { id: 'ai', label: t('nav.workbench'), to: ROUTES.workbench, tone: 'ai', icon: <ChatGlyph className="size-4" />, run: () => go(ROUTES.workbench) },
  ];
  const resources: NavEntry[] = [
    { id: 'skills', label: t('nav.skills'), to: ROUTES.skills, tone: 'resource', icon: <SkillGlyph className="size-4" />, run: () => go(ROUTES.skills) },
    { id: 'mcp', label: t('nav.mcp'), to: ROUTES.mcp, tone: 'tool', icon: <Icon name="globe" />, run: () => go(ROUTES.mcp) },
  ];
  const settings: NavEntry = { id: 'settings', label: t('nav.settings'), to: ROUTES.settings, icon: <SettingsGlyph className="size-4" />, run: () => go(ROUTES.settings) };
  const actions: NavigationAction[] = [
    ...[...primary, ...resources].map(entry => ({ id: entry.id, label: entry.label, icon: entry.icon, group: 'workspace' as const, run: entry.run! })),
    { id: 'single-task', label: t('workbench.single'), group: 'workspace', icon: <Icon name="play" />, run: () => go(ROUTES.singleTask) },
    { id: 'cli-agent', label: t('workbench.agent'), group: 'workspace', icon: <Icon name="code" />, run: () => go(ROUTES.cliAgent) },
    ...tabs.surfaces.filter(surface => surface.kind === 'app').map(surface => ({ id: `surface:${surface.id}`, label: surface.title, group: 'workspace' as const, icon: <Icon name="file" />, run: () => { closeLauncher(); tabs.onActivate(surface.id); } })),
    { id: 'launcher', label: t('nav.modules'), group: 'apps', icon: <Icon name="menu" />, run: openLauncher },
    ...entries.map(entry => ({ id: `app:${entry.moduleId}`, label: entry.displayName, group: 'apps' as const, icon: <Icon name="code" />, disabled: !appUsable(entry), run: () => openApp(entry) })),
    ...(state.activeSurfaceId ? [{ id: 'focus', label: t(state.focusedSurfaceId ? 'shell.restore' : 'shell.focus'), group: 'workspace' as const, icon: <Icon name="copy" />, run: () => toggleFocus(state.activeSurfaceId!) }] : []),
    { id: 'settings', label: settings.label, group: 'settings', icon: settings.icon, run: settings.run! },
  ];
  const shownId = tabs.backgroundSurfaceId;
  const shown = state.surfaces.find(surface => surface.id === shownId);
  const fullBleed = shown?.kind === 'app' && entries.find(entry => entry.moduleId === shown.moduleId)?.layout === 'full-bleed';
  const focused = state.focusedSurfaceId !== null && state.focusedSurfaceId === shownId;
  const pinnedEntries: NavEntry[] = state.pinnedModuleIds.flatMap(id => {
    const entry = entries.find(value => value.moduleId === id);
    return entry ? [{ id, label: entry.displayName, to: appPath(id), tone: 'app' as const, icon: <Icon name={entry.layout === 'full-bleed' ? 'palette' : 'code'} />, run: () => openApp(entry) }] : [];
  });
  const surfaceStatuses = { ...statuses, ai: { dirty: chatDirty, running: chatRunning } };
  return <div className="flex h-screen w-full flex-col">
    <ModuleHostBootstrap />
    <AppShell navEntries={primary} navGroups={[{ eyebrow: t('shell.resources'), entries: resources }]} pinnedEntries={pinnedEntries}
      collapsed={userCollapsed || narrow} autoCollapsed={narrow} onToggleCollapsed={() => desktop.patch({ sidebarCollapsed: !userCollapsed })}
      onOpenLauncher={openLauncher} onPinModule={dropPin} onUnpinModule={unpin} onMovePin={movePin} onDragModule={moduleId => controller.dispatch(moduleId ? { type: 'launcher/drag-start', moduleId } : { type: 'launcher/drag-end' })}
      draggingModuleId={state.launcher.dragModuleId} railHidden={focused || fullBleed} railFooter={<AccountMenu collapsed={userCollapsed || narrow} />} settingsEntry={settings}
      tabs={{ ...tabs, focusedSurfaceId: state.focusedSurfaceId, surfaceStatuses, runningModuleIds: entries.filter(entry => appUsable(entry)).map(entry => entry.moduleId), onToggleFocus: toggleFocus, onOpenLauncher: openLauncher, onOpenCommands: () => setToolsOpen(true) }}>
      <McpConnectionNotice />
      {desktop.error ? <div role="alert" className="flex flex-wrap items-center gap-2 p-2"><span>{t(desktop.error === 'conflict' ? 'home.preferencesConflict' : 'home.preferencesFailed')}</span><Button variant="secondary" disabled={desktop.busy} onClick={() => void desktop.load()}>{t('home.reloadPreferences')}</Button></div> : null}
      <div className="app-content h-full min-h-0 overflow-hidden">
        <div hidden={shownId !== null || (!tabs.settingsOpen && intent?.kind !== 'bare')} inert={shownId !== null || tabs.settingsOpen || state.launcher.open || toolsOpen} className="h-full overflow-auto"><Desktop entries={entries} onOpenApps={openLauncher} onOpenApp={openApp} />{catalogError ? <p role="alert">{t('shell.catalogError')} <Button variant="secondary" onClick={() => void refresh()}>{t('common.retry')}</Button></p> : null}</div>
        {state.surfaces.map(surface => {
          const active = surface.id === shownId;
          return <div key={surface.id} hidden={!active} inert={!active || tabs.settingsOpen || state.launcher.open || toolsOpen} className="h-full min-h-0 overflow-auto" data-workspace-surface={surface.id} data-app-layout={surface.kind === 'app' ? entries.find(entry => entry.moduleId === surface.moduleId)?.layout : undefined}>
            <SurfaceScope id={surface.id} active={active && !tabs.settingsOpen && !state.launcher.open && !toolsOpen} controller={controller}>
              <Suspense fallback={<div role="status">{t('common.loading')}</div>}>
                {surface.kind === 'ai' ? <Workbench /> : surface.kind === 'resource' ? surface.id === '/skills' ? <SkillsManagement /> : <McpManagement /> : surface.kind === 'task' ? surface.id === 'single-task' ? <SingleTask /> : <Agent /> : <AppView surface={surface} available={entries.some(entry => entry.moduleId === surface.moduleId && appUsable(entry))} />}
              </Suspense>
            </SurfaceScope>
          </div>;
        })}
        {!tabs.settingsOpen && intent === null ? <NotFoundRoute /> : null}
        {!tabs.settingsOpen && intent?.kind === 'module' && !shown ? <div className="p-6"><p role="status">{t('shell.unavailable')}</p><Button onClick={settings.run}>{t('shell.openManagement')}</Button></div> : null}
      </div>
      <Launcher open={state.launcher.open} query={state.launcher.query} catalog={entries} loading={!catalog && !catalogError} error={catalogError} notice={launcherNotice} onRetry={() => void refresh()} onManage={() => go(ROUTES.modules)} onQuery={query => controller.dispatch({ type: 'launcher/search', query })} onClose={closeLauncher} onOpen={openApp} onPin={pinApp} onUnpin={unpin} onMove={movePin} pinnedIds={state.pinnedModuleIds} activeDrag={state.launcher.dragModuleId} onDragStart={moduleId => controller.dispatch({ type: 'launcher/drag-start', moduleId })} onDragEnd={() => controller.dispatch({ type: 'launcher/drag-end' })} />
      {tabs.settingsOpen ? <SettingsOverlay controller={controller} onClose={tabs.closeSettings}><Suspense fallback={<div role="status">{t('common.loading')}</div>}><Routes><Route path="/settings/*" caseSensitive element={<SettingsPage />} /></Routes></Suspense></SettingsOverlay> : null}
      <WorkspaceTools actions={actions} open={toolsOpen} onClose={() => setToolsOpen(false)} />
    </AppShell>
  </div>;
}

export default App;

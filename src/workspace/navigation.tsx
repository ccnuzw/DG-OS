import { Icon } from '@dgos/ui';
import { ROUTES } from '../appRoutes';
import type { WorkspaceAppEntry } from '../modules/moduleApi';
import type { NavEntry } from './AppShell';
import type { NavigationAction } from './WorkspaceTools';
import { ChatGlyph, DesktopGlyph, SettingsGlyph, SkillGlyph } from './icons';
import type { WorkspaceSurface } from './model';
import { appPath } from './routes';
import { appUsable } from './useAppCatalog';

type Translate = (key: string) => string;
type Destination = NavEntry & { run: () => void };

export interface BuiltinNavigation {
  primary: Destination[];
  groups: { eyebrow: string; entries: Destination[] }[];
  settings: Destination;
  workspaceActions: NavigationAction[];
}
export interface AppNavigation {
  pinnedEntries: NavEntry[];
  actions: NavigationAction[];
}

// Presentation data only: URL interpretation and surface state stay in routes/tabs/controller.
export function buildBuiltinNavigation(t: Translate, go: (path: string) => void): BuiltinNavigation {
  const primary: Destination[] = [
    { id: 'desktop', label: t('nav.home'), shortLabel: t('nav.homeShort'), to: ROUTES.home, end: true, icon: <DesktopGlyph className="size-4" />, run: () => go(ROUTES.home) },
    { id: 'ai', label: t('nav.workbench'), to: ROUTES.workbench, icon: <ChatGlyph className="size-4" />, run: () => go(ROUTES.workbench) },
  ];
  const resources: Destination[] = [
    { id: 'skills', label: t('nav.skills'), to: ROUTES.skills, icon: <SkillGlyph className="size-4" />, run: () => go(ROUTES.skills) },
    { id: 'mcp', label: t('nav.mcp'), shortLabel: t('nav.mcpShort'), to: ROUTES.mcp, icon: <Icon name="globe" />, run: () => go(ROUTES.mcp) },
  ];
  const settings: Destination = { id: 'settings', label: t('nav.settings'), to: ROUTES.settings, icon: <SettingsGlyph className="size-4" />, run: () => go(ROUTES.settings) };
  const workspaceActions: NavigationAction[] = [
    ...[...primary, ...resources].map(entry => ({ id: entry.id, label: entry.label, icon: entry.icon, group: 'workspace' as const, run: entry.run })),
    { id: 'single-task', label: t('workbench.single'), group: 'workspace', icon: <Icon name="play" />, run: () => go(ROUTES.singleTask) },
    { id: 'cli-agent', label: t('workbench.agent'), group: 'workspace', icon: <Icon name="code" />, run: () => go(ROUTES.cliAgent) },
  ];
  return { primary, groups: [{ eyebrow: t('shell.resources'), entries: resources }], settings, workspaceActions };
}

export function buildAppNavigation(entries: WorkspaceAppEntry[], pinnedIds: string[], openApp: (entry: WorkspaceAppEntry) => void): AppNavigation {
  const pinnedEntries: NavEntry[] = pinnedIds.flatMap(id => {
    const entry = entries.find(value => value.moduleId === id);
    return entry ? [{ id, label: entry.displayName, to: appPath(id), icon: <Icon name={entry.layout === 'full-bleed' ? 'palette' : 'code'} />, run: () => openApp(entry) }] : [];
  });
  const actions: NavigationAction[] = entries.map(entry => ({
    id: `app:${entry.moduleId}`, label: entry.displayName, group: 'apps', icon: <Icon name="code" />,
    disabled: !appUsable(entry), run: () => openApp(entry),
  }));
  return { pinnedEntries, actions };
}

export function buildCommandActions(t: Translate, {
  builtin, apps, surfaces, activeSurfaceId, focusedSurfaceId, openLauncher, activateSurface, toggleFocus,
}: {
  builtin: BuiltinNavigation;
  apps: AppNavigation;
  surfaces: WorkspaceSurface[];
  activeSurfaceId: string | null;
  focusedSurfaceId: string | null;
  openLauncher: () => void;
  activateSurface: (id: string) => void;
  toggleFocus: (id: string) => void;
}): NavigationAction[] {
  return [
    ...builtin.workspaceActions,
    ...surfaces.filter(surface => surface.kind === 'app').map(surface => ({ id: `surface:${surface.id}`, label: surface.title, group: 'workspace' as const, icon: <Icon name="file" />, run: () => activateSurface(surface.id) })),
    { id: 'launcher', label: t('nav.modules'), group: 'apps', icon: <Icon name="menu" />, run: openLauncher },
    ...apps.actions,
    ...(activeSurfaceId ? [{ id: 'focus', label: t(focusedSurfaceId ? 'shell.restore' : 'shell.focus'), group: 'workspace' as const, icon: <Icon name="copy" />, run: () => toggleFocus(activeSurfaceId) }] : []),
    { id: 'settings', label: builtin.settings.label, group: 'settings', icon: builtin.settings.icon, run: builtin.settings.run },
  ];
}

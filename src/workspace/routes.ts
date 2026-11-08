import { AI_SURFACE_ID, type WorkspaceSurface } from './model';

export type WorkspaceIntent =
  | { kind: 'tab'; surfaceId: string; titleKey: string; surfaceKind: 'ai' | 'resource' | 'task' }
  | { kind: 'bare'; view: 'desktop' }
  | { kind: 'overlay'; view: 'settings' }
  | { kind: 'module'; moduleId: string; documentId: string | null };
export interface RouteEntry { path: string; intent: Exclude<WorkspaceIntent, {kind: 'module'}> }
export const WORKSPACE_ROUTES: readonly RouteEntry[] = [
  { path: '/', intent: { kind: 'bare', view: 'desktop' } },
  { path: '/workbench', intent: { kind: 'tab', surfaceId: AI_SURFACE_ID, titleKey: 'nav.workbench', surfaceKind: 'ai' } },
  { path: '/skills', intent: { kind: 'tab', surfaceId: '/skills', titleKey: 'nav.skills', surfaceKind: 'resource' } },
  { path: '/mcp', intent: { kind: 'tab', surfaceId: '/mcp', titleKey: 'nav.mcp', surfaceKind: 'resource' } },
  { path: '/tasks/single', intent: { kind: 'tab', surfaceId: 'single-task', titleKey: 'workbench.single', surfaceKind: 'task' } },
  { path: '/tasks/agent', intent: { kind: 'tab', surfaceId: 'cli-agent', titleKey: 'workbench.agent', surfaceKind: 'task' } },
  { path: '/settings', intent: { kind: 'overlay', view: 'settings' } },
];
export function canonicalPath(path: string): string {
  const legacy: Record<string, string> = { '/settings': '/settings/appearance', '/providers': '/settings/providers', '/account': '/settings/account', '/settings/skills': '/skills', '/settings/mcp': '/mcp' };
  return legacy[path] ?? path;
}
export function intentForPath(pathname: string): WorkspaceIntent | null {
  const path = canonicalPath(pathname);
  const entry = WORKSPACE_ROUTES.find(candidate => candidate.path === path);
  if (entry) return entry.intent;
  if (path.startsWith('/settings/')) return { kind: 'overlay', view: 'settings' };
  const match = /^\/apps\/([^/]+)(?:\/documents\/([^/]+))?$/.exec(path);
  if (match) {
    try { return { kind: 'module', moduleId: decodeURIComponent(match[1]), documentId: match[2] ? decodeURIComponent(match[2]) : null }; } catch { return null; }
  }
  return null;
}
export function pathForSurface(surfaceId: string, surfaces: readonly WorkspaceSurface[] = []): string | null {
  const route = WORKSPACE_ROUTES.find(candidate => candidate.intent.kind === 'tab' && candidate.intent.surfaceId === surfaceId);
  if (route) return route.path;
  const surface = surfaces.find(candidate => candidate.id === surfaceId);
  return surface?.kind === 'app' ? appPath(surface.moduleId, surface.documentId) : null;
}
export function appPath(moduleId: string, documentId: string | null = null): string {
  return `/apps/${encodeURIComponent(moduleId)}${documentId === null ? '' : `/documents/${encodeURIComponent(documentId)}`}`;
}

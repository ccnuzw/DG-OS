// Canonical application destinations used by navigation and error actions.
// Legacy URLs resolve through the workspace's single navigation mapping; they do not own pages.
import { canonicalPath } from './workspace/routes';
export const ROUTES = {
  home: '/',
  skills: '/skills',
  workbench: '/workbench',
  singleTask: '/tasks/single',
  cliAgent: '/tasks/agent',
  mcp: '/mcp',
  settings: '/settings',
  // Settings categories render within the workspace overlay.
  settingsProviders: '/settings/providers',
  modules: '/settings/modules',
  appearance: '/settings/appearance',
  about: '/settings/about',
  lan: '/settings/lan',
  account: '/settings/account',
  cli: '/settings/cli',
  storage: '/settings/storage',
} as const;

const KNOWN: ReadonlySet<string> = new Set(Object.values(ROUTES));

export function routeExists(route: string): boolean {
  return KNOWN.has(canonicalPath(route));
}

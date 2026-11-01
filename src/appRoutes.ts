// The application's route table, as far as error actions need it (FR-005 §8.3, P7).
//
// `handleError` only offers a navigation button when `routeExists` says the target route is real, so the
// answer must come from the same place that declares the routes. App.tsx renders its <Route> elements from
// these paths; a route added there without being listed here simply gets no error action.
export const ROUTES = {
  home: '/',
  providers: '/providers',
  skills: '/skills',
  workbench: '/workbench',
  mcp: '/mcp',
  settings: '/settings',
  // V1-FR-007 settings sections (D085). The Provider, Skills and MCP sections render the same pages as the
  // top-level routes above.
  settingsProviders: '/settings/providers',
  settingsSkills: '/settings/skills',
  settingsMcp: '/settings/mcp',
  modules: '/settings/modules',
  appearance: '/settings/appearance',
  about: '/settings/about',
  lan: '/settings/lan',
  account: '/account',
} as const;

const KNOWN: ReadonlySet<string> = new Set(Object.values(ROUTES));

export function routeExists(route: string): boolean {
  return KNOWN.has(route);
}

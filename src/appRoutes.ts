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
  modules: '/settings/modules',
  appearance: '/settings/appearance',
} as const;

const KNOWN: ReadonlySet<string> = new Set(Object.values(ROUTES));

export function routeExists(route: string): boolean {
  return KNOWN.has(route);
}

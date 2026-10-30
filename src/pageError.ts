import { useCallback } from 'react';
import { useInRouterContext, useNavigate } from 'react-router-dom';
import { handleError } from '@dgos/ui';
import { routeExists } from './appRoutes';

/**
 * Binds `handleError` to one page: `where` names the page in the log line, `navigate` comes from the router
 * and `routeExists` from the application's route table, so an error action appears only for a real route.
 * Without a router (a page rendered on its own) no action button is offered.
 *
 * The router check is constant for the lifetime of a mounted tree, so the conditional hook call is stable.
 */
export function usePageError(where: string): (error: unknown) => void {
  const inRouter = useInRouterContext();
  const navigate = inRouter ? useNavigate() : undefined;
  return useCallback(
    (error: unknown) => handleError(error, { where, navigate, routeExists: navigate ? routeExists : undefined }),
    [where, navigate],
  );
}

import { useContext, useEffect, useRef } from 'react';
import { Link, UNSAFE_LocationContext, useInRouterContext, type LinkProps } from 'react-router-dom';
import { cn } from '@dgos/ui';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { useLocale } from '../../i18n';
import type { NativeSelection } from '../../types/nativeExtensions';
import { useSurfaceActive } from '../../workspace/SurfaceContext';

export function WorkspaceLink(props: LinkProps) {
  const routed = useInRouterContext();
  const active = useSurfaceActive();
  const returnFocus = useRef<HTMLAnchorElement | null>(null);
  useEffect(() => {
    const target = returnFocus.current;
    if (!active || !target) return;
    returnFocus.current = null;
    if (target.isConnected && !target.closest('[hidden], [inert]') && (document.activeElement === document.body || document.activeElement?.closest('[hidden], [inert]'))) target.focus({ preventScroll: true });
  }, [active]);
  const className = cn('text-accent-default underline text-sm wrap-anywhere', FOCUS_RING);
  return routed ? <Link {...props} className={className} onClick={event => { props.onClick?.(event); if (!event.defaultPrevented && !event.metaKey && !event.ctrlKey && !event.shiftKey && !event.altKey) returnFocus.current = event.currentTarget; }} /> : <a href={typeof props.to === 'string' ? props.to : props.to.pathname} className={className}>{props.children}</a>;
}

export function RuntimeNavigation({ runtime }: { runtime: 'dgos' | 'native' }) {
  const { t } = useLocale();
  return <nav className="flex flex-wrap items-center gap-3 p-2" aria-label={t('workbenchNavigation.runtime')}>
    <WorkspaceLink to="/workbench" aria-current={runtime === 'dgos' ? 'page' : undefined}>{t('workbenchNavigation.dgos')}</WorkspaceLink>
    <WorkspaceLink to="/tasks/agent" aria-current={runtime === 'native' ? 'page' : undefined}>{t('workbenchNavigation.native')}</WorkspaceLink>
  </nav>;
}

export function CapabilityLinks({ engine, projectId, sessionId }: { engine?: 'codex' | 'claude'; projectId?: string | null; sessionId?: string }) {
  const { t } = useLocale();
  const nativeSelection: NativeSelection | null = engine ? { target: engine === 'codex' ? 'Codex' : 'Claude', scope: projectId ? 'ProjectShared' : 'OsUser', project_id: projectId ?? null } : null;
  const state = { capabilityEntry: { runtime: engine ? 'native' : 'dgos', selection: nativeSelection, sessionId } };
  return <div className="flex min-w-0 flex-col gap-2">
    <nav className="flex flex-wrap items-center gap-3" aria-label={t('workbenchNavigation.capabilities')}>
      <WorkspaceLink to="/settings/providers">{t('capabilities.provider')}</WorkspaceLink>
      <WorkspaceLink to="/skills" state={state}>{t('nav.skills')}</WorkspaceLink>
      <WorkspaceLink to="/mcp" state={state}>{t('nav.mcp')}</WorkspaceLink>
      <details className="min-w-0 text-sm"><summary className={cn('cursor-pointer', FOCUS_RING)}>{t('workbenchNavigation.permissions')}</summary>
        <p className="m-0 mt-2 text-xs text-ink-secondary wrap-anywhere">{t(engine ? engine === 'codex' ? 'agent.codexPolicy' : 'agent.claudePolicy' : 'workbenchNavigation.hostPermissions')}</p>
        {engine && <p className="m-0 mt-2 text-xs text-ink-secondary wrap-anywhere">{t('capabilities.nativeBoundary')}</p>}
      </details>
    </nav>
    <p className="m-0 text-xs text-ink-secondary wrap-anywhere">{t(engine ? 'workbenchNavigation.nativeSource' : 'workbenchNavigation.dgosSource')}</p>
  </div>;
}

// Navigation context only; the host still validates every target and project. Reading it
// does not select/resume a runtime or execute a management operation.
export function useCapabilityEntry(path: '/skills' | '/mcp') {
  const context = useContext(UNSAFE_LocationContext);
  const location = context?.location;
  const previous = useRef<{ selection?: NativeSelection; sessionId?: string }>({});
  const value = location?.pathname === path ? location.state?.capabilityEntry : null;
  const selection = value?.selection;
  const nativeSelection: NativeSelection | undefined = value?.runtime === 'native' && (selection?.target === 'Codex' || selection?.target === 'Claude') && ['OsUser', 'ProjectShared', 'ProjectLocal'].includes(selection.scope) && (selection.project_id === null || typeof selection.project_id === 'string') ? selection : undefined;
  const sessionId = nativeSelection && typeof value?.sessionId === 'string' ? value.sessionId : undefined;
  if (nativeSelection) previous.current = { selection: nativeSelection, sessionId };
  return { key: location?.pathname === path ? location.key : null, runtime: value?.runtime === 'dgos' ? 'dgos' : nativeSelection ? 'native' : null, nativeSelection: nativeSelection ?? previous.current.selection, sessionId: nativeSelection ? sessionId : previous.current.sessionId };
}

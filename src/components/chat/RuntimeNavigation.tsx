import { useContext, useEffect, useRef, useState } from 'react';
import { Link, UNSAFE_LocationContext, useInRouterContext, useNavigate, type LinkProps } from 'react-router-dom';
import { Button, cn } from '@dgos/ui';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
import { useLocale } from '../../i18n';
import type { NativeSelection } from '../../types/nativeExtensions';
import { useSurfaceActive } from '../../workspace/SurfaceContext';
import { useDevelopment } from '../../stores/developmentStore';
import { chooseContext } from '../../stores/developmentSelection';

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

// Navigation context only; the host still validates every target and project. Reading it
// does not select/resume a runtime or execute a management operation.
export function useCapabilityEntry(path: '/skills' | '/mcp') {
  const context = useContext(UNSAFE_LocationContext);
  const location = context?.location;
  const epoch = useDevelopment(state => state.epoch);
  const previous = useRef<{ epoch: number; selection?: NativeSelection; sessionId?: string; returnTabId?: string }>({ epoch });
  if (previous.current.epoch !== epoch) previous.current = { epoch };
  const candidate = location?.pathname === path ? location.state?.capabilityEntry : null;
  const value = candidate && (candidate.epoch === undefined || candidate.epoch === epoch) ? candidate : null;
  const selection = value?.selection;
  const nativeSelection: NativeSelection | undefined = value?.runtime === 'native' && (selection?.target === 'Codex' || selection?.target === 'Claude') && ['OsUser', 'ProjectShared', 'ProjectLocal'].includes(selection.scope) && (selection.project_id === null || typeof selection.project_id === 'string') ? selection : undefined;
  const sessionId = nativeSelection && typeof value?.sessionId === 'string' ? value.sessionId : undefined;
  const returnTabId = nativeSelection && typeof value?.returnTabId === 'string' ? value.returnTabId : undefined;
  if (nativeSelection) previous.current = { epoch, selection: nativeSelection, sessionId, returnTabId };
  return { key: location?.pathname === path ? location.key : null, runtime: value?.runtime === 'dgos' ? 'dgos' : nativeSelection ? 'native' : null, nativeSelection: nativeSelection ?? previous.current.selection, sessionId: nativeSelection ? sessionId : previous.current.sessionId, returnTabId: nativeSelection ? returnTabId : previous.current.returnTabId };
}

export function CapabilityReturn({ path }: { path: '/skills' | '/mcp' }) {
  const { t } = useLocale();
  const entry = useCapabilityEntry(path);
  const workspace = useDevelopment();
  const navigate = useNavigate();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const tab = workspace.tabs.find(item => item.id === entry.returnTabId);
  if (!entry.returnTabId) return null;
  async function back() {
    if (!tab || busy) return;
    const epoch = workspace.epoch;
    setBusy(true); setFailed(false);
    const selected = await chooseContext({ project_id: tab.projectId, task_id: tab.taskId, session_id: tab.sessionId });
    if (epoch !== useDevelopment.getState().epoch) return;
    setBusy(false);
    if (!selected.ok || !useDevelopment.getState().tabs.some(item => item.id === tab.id)) { setFailed(true); return; }
    useDevelopment.getState().activateTab(tab.id);
    navigate('/tasks/agent');
  }
  return <div className="flex flex-wrap items-center gap-2"><Button size="xs" variant="secondary" disabled={!tab || busy} onClick={() => void back()}>{t('agentControls.returnConversation')}</Button>{!tab || failed ? <p role="status" className="m-0 text-xs">{t('agentControls.returnUnavailable')}</p> : null}</div>;
}

import { useEffect, useRef } from 'react';
import { Link, useInRouterContext, type LinkProps } from 'react-router-dom';
import { cn } from '@dgos/ui';
import { FOCUS_RING } from '@dgos/ui/lib/focus';
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

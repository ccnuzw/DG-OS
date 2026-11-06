import { createContext, useCallback, useContext, useEffect, useMemo, useRef, type ReactNode } from 'react';
import type { SurfaceStatus, WorkspaceController } from './controller';

interface SurfaceContextValue { id: string; active: boolean; visible: boolean; controller: WorkspaceController; register: (guard: () => Promise<boolean>) => () => void }
const SurfaceContext = createContext<SurfaceContextValue | null>(null);
/** Mounted surface interaction eligibility, not task lifetime or visibility of every inner panel. */
export function SurfaceScope({ id, active, visible = active, controller, children }: { id: string; active: boolean; visible?: boolean; controller: WorkspaceController; children: ReactNode }) {
  // Activity changes must not unregister guards while an asynchronous close is walking them.
  const register = useCallback((guard: () => Promise<boolean>) => controller.registerGuard(id, guard), [controller, id]);
  const value = useMemo(() => ({ id, active, visible, controller, register }), [id, active, visible, controller, register]);
  return <SurfaceContext.Provider value={value}>{children}</SurfaceContext.Provider>;
}
/** Defaults to active for standalone consumers; inner tabs must also respect their own selection. */
export function useSurfaceActive(): boolean { return useContext(SurfaceContext)?.active ?? true; }
/** Layout visibility is independent of interaction eligibility under an overlay. */
export function useSurfaceVisible(): boolean { return useContext(SurfaceContext)?.visible ?? true; }
export function useSurfaceCloseGuard(guard: () => Promise<boolean>): void {
  const surface = useContext(SurfaceContext);
  const latest = useRef(guard);
  latest.current = guard;
  useEffect(() => surface?.register(() => latest.current()), [surface?.id, surface?.register]);
}
export function useSurfaceStatus(status: SurfaceStatus): void {
  const surface = useContext(SurfaceContext);
  const owner = useRef({});
  useEffect(() => {
    surface?.controller.setSurfaceStatus(surface.id, owner.current, status);
    return () => surface?.controller.setSurfaceStatus(surface.id, owner.current, null);
  }, [surface?.controller, surface?.id, status.dirty, status.running, status.runningLabel]);
}

import { createContext, useContext, useEffect, useRef, type ReactNode } from 'react';
import type { SurfaceStatus, WorkspaceController } from './controller';

interface SurfaceContextValue { id: string; active: boolean; controller: WorkspaceController; register: (guard: () => Promise<boolean>) => () => void }
const SurfaceContext = createContext<SurfaceContextValue | null>(null);
export function SurfaceScope({ id, active, controller, children }: { id: string; active: boolean; controller: WorkspaceController; children: ReactNode }) {
  return <SurfaceContext.Provider value={{ id, active, controller, register: guard => controller.registerGuard(id, guard) }}>{children}</SurfaceContext.Provider>;
}
export function useSurfaceActive(): boolean { return useContext(SurfaceContext)?.active ?? true; }
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
  }, [surface?.controller, surface?.id, status.dirty, status.running]);
}

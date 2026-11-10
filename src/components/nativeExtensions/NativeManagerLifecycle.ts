import { createContext, useContext, useEffect, useRef } from 'react';

export type NativeManagerGuard = (reason?: 'close' | 'context' | 'view') => boolean | Promise<boolean>;
export const NativeManagerLifecycle = createContext<((guard: NativeManagerGuard) => () => void) | null>(null);

/** Participate in manager dismissal without changing the lifetime of the CLI session. */
export function useNativeManagerGuard(guard: NativeManagerGuard): void {
  const register = useContext(NativeManagerLifecycle);
  const latest = useRef(guard);
  latest.current = guard;
  useEffect(() => register?.(reason => latest.current(reason)), [register]);
}

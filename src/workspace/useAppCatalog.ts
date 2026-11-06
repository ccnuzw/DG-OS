import { useCallback, useEffect, useRef, useState } from 'react';
import { readWorkspaceAppCatalog, type WorkspaceAppCatalog, type WorkspaceAppEntry } from '../modules/moduleApi';
import { useModuleInstances } from '../modules/ModuleSlots';

export const appUsable = (entry: WorkspaceAppEntry) => entry.enabled && entry.permissionState === 'granted' && entry.runtimeState === 'ready' && !entry.crashed;

export function useAppCatalog(pathname: string) {
  const [catalog, setCatalog] = useState<WorkspaceAppCatalog | null>(null);
  const [error, setError] = useState(false);
  const sequence = useRef(0);
  const instances = useModuleInstances();
  const refresh = useCallback(async () => {
    const ticket = ++sequence.current;
    try {
      const value = await readWorkspaceAppCatalog();
      if (ticket === sequence.current) { setCatalog(value); setError(false); }
    } catch { if (ticket === sequence.current) { setError(true); setCatalog(null); } }
  }, []);
  useEffect(() => { void refresh(); return () => { ++sequence.current; }; }, [refresh, pathname, instances]);
  useEffect(() => {
    const invalidate = () => { void refresh(); };
    window.addEventListener('dgos:catalog-invalidated', invalidate);
    return () => window.removeEventListener('dgos:catalog-invalidated', invalidate);
  }, [refresh]);
  return { catalog, error, refresh };
}

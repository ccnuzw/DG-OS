import { useCallback, useSyncExternalStore } from 'react';
import { moduleRuntimeHost, type ModuleInstanceView } from './moduleRuntimeHost';

// The container is owned by the runtime host. Moving it here never recreates it, and when this
// wrapper goes away (route change) it is only detached: the module root keeps living (D044).
export function ModuleSlot({ instance }: { instance: ModuleInstanceView }) {
  const { container } = instance;
  // Stable per container so unrelated re-renders do not detach and re-append the module DOM.
  const attach = useCallback(
    (wrapper: HTMLDivElement | null) => {
      if (wrapper === null) return;
      wrapper.appendChild(container);
      return () => {
        if (container.parentNode === wrapper) wrapper.removeChild(container);
      };
    },
    [container],
  );
  return <div data-testid="module-slot" data-runtime-id={instance.runtimeId} ref={attach} />;
}

export function useModuleInstances(): readonly ModuleInstanceView[] {
  return useSyncExternalStore(moduleRuntimeHost.subscribe, moduleRuntimeHost.getSnapshot);
}

// During a hot update the old and the new instance briefly coexist; the newest one is the live one.
export function useModuleInstance(moduleId: string): ModuleInstanceView | null {
  const instances = useModuleInstances();
  const matches = instances.filter((instance) => instance.moduleId === moduleId);
  return matches[matches.length - 1] ?? null;
}

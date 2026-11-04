import { useSyncExternalStore } from 'react';
import { initialWorkspaceState, type LayoutSnapshot, type WorkspaceState, type WorkspaceSurface } from './model';
import { reduce, type WorkspaceAction, type ReduceOutcome } from './reducer';

export interface SurfaceStatus { dirty?: boolean; running?: boolean }
export interface WorkspaceController {
  getSnapshot(): WorkspaceState;
  subscribe(listener: () => void): () => void;
  dispatch(action: WorkspaceAction): ReduceOutcome;
  open(surface: WorkspaceSurface): ReduceOutcome;
  registerGuard(id: string, guard: () => Promise<boolean>): () => void;
  canLeave(id: string): Promise<boolean>;
  requestClose(id: string): Promise<boolean>;
  getLayout(): LayoutSnapshot;
  setLayout(value: LayoutSnapshot): void;
  toggleFocus(surfaceId: string): ReduceOutcome;
  setSurfaceStatus(id: string, owner: object, status: SurfaceStatus | null): void;
  getSurfaceStatuses(): Readonly<Record<string, SurfaceStatus>>;
  subscribeSurfaceStatuses(listener: () => void): () => void;
  reset(): void;
}

export function createWorkspaceController(): WorkspaceController {
  let state = initialWorkspaceState();
  const listeners = new Set<() => void>();
  const guards = new Map<string, Set<() => Promise<boolean>>>();
  const closing = new Map<string, Promise<boolean>>();
  const statusOwners = new Map<string, Map<object, SurfaceStatus>>();
  let statuses: Readonly<Record<string, SurfaceStatus>> = {};
  const statusListeners = new Set<() => void>();
  let layout: LayoutSnapshot = { sidebar: true, inspector: false, history: false };
  const canLeave = async (id: string) => {
    for (const guard of guards.get(id) ?? []) if (!await guard()) return false;
    return true;
  };
  const dispatch = (action: WorkspaceAction) => {
    const outcome = reduce(state, action);
    state = outcome.state;
    if (outcome.restoreLayout) layout = outcome.restoreLayout;
    for (const listener of listeners) listener();
    return outcome;
  };
  return {
    getSnapshot: () => state,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    dispatch,
    open: (surface: WorkspaceSurface) => dispatch({ type: 'surface/open', surface }),
    registerGuard(id: string, guard: () => Promise<boolean>) {
      const set = guards.get(id) ?? new Set<() => Promise<boolean>>();
      set.add(guard); guards.set(id, set);
      return () => { set.delete(guard); if (!set.size) guards.delete(id); };
    },
    canLeave,
    requestClose(id: string): Promise<boolean> {
      const pending = closing.get(id);
      if (pending) return pending;
      const epoch = state.accountEpoch;
      const target = state.surfaces.find(surface => surface.id === id);
      if (!target) return Promise.resolve(true);
      const operation = Promise.resolve().then(async () => {
        if (!await canLeave(id)) return false;
        if (state.accountEpoch !== epoch || !state.surfaces.includes(target)) return false;
        dispatch({ type: 'surface/close', surfaceId: id });
        return true;
      }).finally(() => { if (closing.get(id) === operation) closing.delete(id); });
      closing.set(id, operation);
      return operation;
    },
    getLayout: () => layout,
    setLayout(value: LayoutSnapshot) { layout = value; for (const listener of listeners) listener(); },
    toggleFocus(surfaceId: string) { return dispatch({ type: 'surface/focus-toggle', surfaceId, layout }); },
    setSurfaceStatus(id, owner, status) {
      const owners = statusOwners.get(id) ?? new Map<object, SurfaceStatus>();
      if (status) owners.set(owner, status); else owners.delete(owner);
      if (owners.size) statusOwners.set(id, owners); else statusOwners.delete(id);
      const dirty = [...owners.values()].some(value => value.dirty);
      const running = [...owners.values()].some(value => value.running);
      if (statuses[id]?.dirty === dirty && statuses[id]?.running === running) return;
      statuses = { ...statuses, [id]: { dirty, running } };
      for (const listener of statusListeners) listener();
    },
    getSurfaceStatuses: () => statuses,
    subscribeSurfaceStatuses(listener) { statusListeners.add(listener); return () => { statusListeners.delete(listener); }; },
    reset() { guards.clear(); closing.clear(); statusOwners.clear(); statuses = {}; for (const listener of statusListeners) listener(); dispatch({ type: 'account/reset', epoch: state.accountEpoch + 1 }); },
  };
}
export function useWorkspaceState(controller: WorkspaceController): WorkspaceState {
  return useSyncExternalStore(controller.subscribe, controller.getSnapshot, controller.getSnapshot);
}

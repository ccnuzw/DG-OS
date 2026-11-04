/*
 * V2-FR-009 workspace reducer (D120, D124, D121, D122).
 *
 * Pure and serial: every transition returns a new state and never mutates the input, so one reducer call is
 * the whole transaction boundary (technical design §并发控制, §事务边界). Opening an app never starts or
 * stops a module — the host owns runtime lifecycle (D082), and closing a tab only detaches the view.
 */
import {
  AI_SURFACE_ID,
  MAX_PINNED,
  findSurface,
  initialWorkspaceState,
  viewIdFor,
  type AppInstanceState,
  type LayoutSnapshot,
  type WorkspaceState,
  type WorkspaceSurface,
} from './model';

/** What an open/close asked for, so the caller can react (focus the tab, show a message, ask the host). */
export type WorkspaceResult =
  | 'created'
  | 'focused'
  | 'waiting'
  | 'blocked'
  | 'needs-management'
  | 'closed'
  | 'already-closed'
  | 'pinned'
  | 'unpinned'
  | 'already-pinned'
  | 'limit'
  | 'focused-layout'
  | 'restored-layout'
  | 'reset'
  | 'noop';

export type OpenAppAction = {
  type: 'app/open';
  moduleId: string;
  title: string;
  documentId: string | null;
  /** Catalog says the module is enabled and permitted. */
  usable: boolean;
  /** Host reports a live runtimeId for the module. */
  runtimeReady: boolean;
  /** The account generation the caller observed; a stale action is refused (B09-01). */
  epoch?: number;
};

export type WorkspaceAction =
  | OpenAppAction
  | { type: 'ai/open' }
  | { type: 'surface/open'; surface: WorkspaceSurface }
  | { type: 'surface/deactivate' }
  | { type: 'surface/activate'; surfaceId: string }
  | { type: 'surface/close'; surfaceId: string }
  | { type: 'surface/focus-toggle'; surfaceId: string; layout: LayoutSnapshot }
  | { type: 'app/pin'; moduleId: string }
  | { type: 'app/unpin'; moduleId: string }
  | { type: 'app/move-pin'; moduleId: string; targetIndex: number }
  | { type: 'preferences/hydrate'; pinnedModuleIds: string[] }
  | { type: 'launcher/open' }
  | { type: 'launcher/close' }
  | { type: 'launcher/search'; query: string }
  | { type: 'launcher/status'; status: WorkspaceState['launcher']['status'] }
  | { type: 'launcher/drag-start'; moduleId: string }
  | { type: 'launcher/drag-end' }
  | { type: 'launcher/drop-to-pin'; moduleId: string; target: 'none' | 'sidebar' }
  | { type: 'instances/reconcile'; instances: AppInstanceState[] }
  | { type: 'account/reset'; epoch: number };

export type ReduceOutcome = { state: WorkspaceState; result: WorkspaceResult; restoreLayout?: LayoutSnapshot };

const noop = (state: WorkspaceState, result: WorkspaceResult = 'noop'): ReduceOutcome => ({ state, result });

function activate(state: WorkspaceState, surfaceId: string): WorkspaceState {
  return { ...state, activeSurfaceId: surfaceId };
}

function openApp(state: WorkspaceState, action: OpenAppAction): ReduceOutcome {
  if (action.epoch !== undefined && action.epoch !== state.accountEpoch) return noop(state, 'blocked');
  if (!action.usable) return noop(state, 'needs-management');

  // Reopening the same thing focuses it instead of creating a second view (B09-03, B09-05).
  const existing = state.surfaces.find(
    (surface) =>
      surface.kind === 'app' &&
      surface.moduleId === action.moduleId &&
      (action.documentId === null || surface.documentId === action.documentId),
  );
  if (existing) return { state: activate(state, existing.id), result: 'focused' };

  // A double click while the previous open is still resolving must not create two views (B09-04).
  if (state.pending[action.moduleId] === 'opening') return noop(state, 'waiting');

  // No live runtime: the app cannot be shown yet; management is where it gets started (B09-07).
  if (!action.runtimeReady) return noop(state, 'needs-management');

  const viewId = viewIdFor(action.moduleId, action.documentId);
  const surface: WorkspaceSurface = {
    kind: 'app',
    id: viewId,
    moduleId: action.moduleId,
    viewId,
    documentId: action.documentId,
    title: action.title,
  };
  return {
    state: {
      ...state,
      surfaces: [...state.surfaces, surface],
      activeSurfaceId: surface.id,
      instances: withView(state.instances, action.moduleId, viewId),
    },
    result: 'created',
  };
}

function withView(
  instances: Record<string, AppInstanceState>,
  moduleId: string,
  viewId: string,
): Record<string, AppInstanceState> {
  const current = instances[moduleId];
  if (!current) return instances;
  if (current.views.includes(viewId)) return instances;
  return { ...instances, [moduleId]: { ...current, views: [...current.views, viewId] } };
}

function withoutView(
  instances: Record<string, AppInstanceState>,
  moduleId: string,
  viewId: string,
): Record<string, AppInstanceState> {
  const current = instances[moduleId];
  if (!current || !current.views.includes(viewId)) return instances;
  return { ...instances, [moduleId]: { ...current, views: current.views.filter((id) => id !== viewId) } };
}

function closeSurface(state: WorkspaceState, surfaceId: string): ReduceOutcome {
  const index = state.surfaces.findIndex((surface) => surface.id === surfaceId);
  if (index === -1) return noop(state, 'already-closed');

  const target = state.surfaces[index];
  const surfaces = state.surfaces.filter((surface) => surface.id !== surfaceId);

  // Closing a tab detaches the view only; the module keeps running in the host (D082, B09-10/B09-11).
  const instances =
    target.kind === 'app' ? withoutView(state.instances, target.moduleId, target.id) : state.instances;

  let activeSurfaceId = state.activeSurfaceId;
  if (state.activeSurfaceId === surfaceId) {
    // Prefer the tab that slid into this position, else the one before it (B09-12).
    const neighbour = surfaces[index] ?? surfaces[index - 1];
    activeSurfaceId = neighbour ? neighbour.id : null;
  }

  const focusedSurfaceId = state.focusedSurfaceId === surfaceId ? null : state.focusedSurfaceId;
  const layoutSnapshot = focusedSurfaceId === null ? null : state.layoutSnapshot;

  return {
    state: { ...state, surfaces, instances, activeSurfaceId, focusedSurfaceId, layoutSnapshot },
    ...(state.focusedSurfaceId === surfaceId && state.layoutSnapshot ? { restoreLayout: state.layoutSnapshot } : {}),
    result: 'closed',
  };
}

function pin(state: WorkspaceState, moduleId: string): ReduceOutcome {
  if (state.pinnedModuleIds.includes(moduleId)) return noop(state, 'already-pinned');
  if (state.pinnedModuleIds.length >= MAX_PINNED) return noop(state, 'limit');
  return {
    state: { ...state, pinnedModuleIds: [...state.pinnedModuleIds, moduleId] },
    result: 'pinned',
  };
}

function focusToggle(state: WorkspaceState, surfaceId: string, layout: LayoutSnapshot): ReduceOutcome {
  if (state.focusedSurfaceId === surfaceId) {
    return {
      state: { ...state, focusedSurfaceId: null, layoutSnapshot: null },
      result: 'restored-layout',
      ...(state.layoutSnapshot ? { restoreLayout: state.layoutSnapshot } : {}),
    };
  }
  return {
    state: { ...state, focusedSurfaceId: surfaceId, layoutSnapshot: state.layoutSnapshot ?? { ...layout } },
    result: 'focused-layout',
  };
}

export function reduce(state: WorkspaceState, action: WorkspaceAction): ReduceOutcome {
  switch (action.type) {
    case 'preferences/hydrate':
      return { state: { ...state, pinnedModuleIds: [...action.pinnedModuleIds] }, result: 'noop' };
    case 'surface/open': {
      const exists = findSurface(state, action.surface.id);
      return { state: { ...state, surfaces: exists ? state.surfaces : [...state.surfaces, action.surface], activeSurfaceId: action.surface.id }, result: exists ? 'focused' : 'created' };
    }
    case 'surface/deactivate':
      return { state: { ...state, activeSurfaceId: null }, result: 'noop' };
    case 'app/open':
      return openApp(state, action);

    case 'ai/open': {
      const existing = findSurface(state, AI_SURFACE_ID);
      if (existing) return { state: activate(state, AI_SURFACE_ID), result: 'focused' };
      const surface: WorkspaceSurface = {
        kind: 'ai',
        id: AI_SURFACE_ID,
      };
      return {
        state: { ...state, surfaces: [...state.surfaces, surface], activeSurfaceId: AI_SURFACE_ID },
        result: 'created',
      };
    }

    case 'surface/activate': {
      if (!findSurface(state, action.surfaceId)) return noop(state, 'noop');
      return { state: activate(state, action.surfaceId), result: 'focused' };
    }

    case 'surface/close':
      return closeSurface(state, action.surfaceId);

    case 'surface/focus-toggle':
      return focusToggle(state, action.surfaceId, action.layout);

    case 'app/pin':
      return pin(state, action.moduleId);

    case 'app/unpin': {
      if (!state.pinnedModuleIds.includes(action.moduleId)) return noop(state, 'noop');
      return {
        state: {
          ...state,
          pinnedModuleIds: state.pinnedModuleIds.filter((id) => id !== action.moduleId),
        },
        result: 'unpinned',
      };
    }

    case 'app/move-pin': {
      const index = state.pinnedModuleIds.indexOf(action.moduleId);
      if (index < 0 || action.targetIndex < 0 || action.targetIndex >= state.pinnedModuleIds.length || index === action.targetIndex) return noop(state);
      const pinnedModuleIds = [...state.pinnedModuleIds];
      pinnedModuleIds.splice(index, 1);
      pinnedModuleIds.splice(action.targetIndex, 0, action.moduleId);
      return { state: { ...state, pinnedModuleIds }, result: 'pinned' };
    }

    case 'launcher/open':
      if (state.launcher.open) return noop(state, 'focused');
      return {
        state: { ...state, launcher: { ...state.launcher, open: true, status: 'loading' } },
        result: 'created',
      };

    case 'launcher/close':
      // Dismissing the panel never closes a work surface; it only clears transient launcher state.
      return {
        state: {
          ...state,
          launcher: { ...state.launcher, open: false, query: '', status: 'idle', dragModuleId: null, dropTarget: 'none' },
        },
        result: 'closed',
      };

    case 'launcher/search':
      return { state: { ...state, launcher: { ...state.launcher, query: action.query } }, result: 'noop' };

    case 'launcher/status':
      return { state: { ...state, launcher: { ...state.launcher, status: action.status } }, result: 'noop' };

    case 'launcher/drag-start':
      if (!state.launcher.open && !state.pinnedModuleIds.includes(action.moduleId)) return noop(state, 'blocked');
      return {
        state: { ...state, launcher: { ...state.launcher, dragModuleId: action.moduleId, dropTarget: 'sidebar' } },
        result: 'noop',
      };

    case 'launcher/drag-end':
      return {
        state: { ...state, launcher: { ...state.launcher, dragModuleId: null, dropTarget: 'none' } },
        result: 'noop',
      };

    case 'launcher/drop-to-pin': {
      if (state.launcher.dragModuleId !== action.moduleId || action.target !== 'sidebar') {
        return noop(state, 'noop');
      }
      const cleared: WorkspaceState = {
        ...state,
        launcher: { ...state.launcher, dragModuleId: null, dropTarget: 'none' },
      };
      const outcome = pin(cleared, action.moduleId);
      return outcome;
    }

    case 'instances/reconcile': {
      const instances: Record<string, AppInstanceState> = {};
      for (const instance of action.instances) instances[instance.moduleId] = instance;
      return { state: { ...state, instances }, result: 'noop' };
    }

    case 'account/reset':
      return { state: initialWorkspaceState(action.epoch), result: 'reset' };

    default:
      return noop(state);
  }
}
